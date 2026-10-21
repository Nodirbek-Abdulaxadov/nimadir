//! The WASM Component-Model boundary: embeds Wasmtime, implements the typed host
//! interface generated from `wit/`, and drives one guest frame at a time.
//!
//! The mini-app is now a **component**, and the host<->guest contract is the
//! `wit/` package — not a hand-rolled `(ptr, len)` ABI. `bindgen!` turns that
//! WIT into the `Host` trait we implement below (the capability table) and the
//! `MiniAppWorld` we instantiate. The canonical ABI moves strings/bools across
//! the boundary, so there is no manual linear-memory reading here any more.
//!
//! The public API of `MiniApp` (`load` / `frame` / `take_logs` /
//! `take_pending_open`) is all `shell.rs` drives; everything Wasmtime-specific —
//! the bindings, the linker, the time and memory limits — stays in this file.

use std::time::Duration;

use anyhow::{anyhow, Result};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store, Trap};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::dom::{self, Document};
use crate::images::Images;
use crate::net::{self, Net};
use crate::registry::AppEntry;
use crate::storage::Storage;
use crate::ui::{Container, FrameInput, UiCmd};

/// Generated bindings for the `mini-app` world, kept in a submodule so the
/// generated world type (`bindings::MiniApp`) doesn't clash with our own
/// `MiniApp` wrapper below.
mod bindings {
    wasmtime::component::bindgen!({
        world: "mini-app",
        path: "../wit",
        // Every import may trap, so a capability can stop a guest that
        // overspends it (see `HostState::spend`) instead of only refusing it.
        imports: { default: trappable },
    });
}

use bindings::nimadir::shell::dom::{Event as WitEvent, Host as DomHost, Rect};
use bindings::nimadir::shell::host_api::{
    AppEntry as WitAppEntry, Host, HttpRequest, HttpResponse,
};
use bindings::MiniApp as MiniAppWorld;

/// How often the engine's epoch advances. Every limit on guest *time* below is
/// counted in these ticks, so this is also the precision of those limits.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// How long one `update` may run: about a second.
///
/// A frame is meant to take microseconds. A guest still inside `update` after
/// a second is not slow, it is stuck — and it is stuck *on the UI thread*,
/// where a `loop {}` used to freeze the whole window for good (and on Android
/// would earn an "app not responding" kill for the shell, not the guest).
const FRAME_BUDGET_TICKS: u64 = 100;

/// How long instantiation plus `init` may run: about ten seconds. Far more
/// than a frame, because a guest that carries a runtime boots it here — CPython
/// is the slow one — but still finite, since this runs on a loader thread that
/// nothing could otherwise ever reclaim.
const INIT_BUDGET_TICKS: u64 = 1000;

/// The most linear memory one guest may hold: 512 MiB.
///
/// Without a cap a guest could grow to the 4 GiB a 32-bit memory can address,
/// on a machine that may not have it — the host's allocation fails, not the
/// guest's. Generous on purpose: CPython inside the sandbox needs room.
pub const MAX_GUEST_MEMORY: usize = 512 << 20;

/// The most text one frame may send the host — widget text and log lines
/// together: 16 MiB.
///
/// The memory cap above bounds what a guest holds, not what it hands over:
/// every `ui-label` copies its string into a `Vec` on the *host*, so a guest
/// looping for its one-second budget could fill the host's memory with copies
/// of a buffer it only allocated once. Far beyond any real frame — a store of
/// 500 apps sends well under 100 KB.
pub const MAX_FRAME_BYTES: usize = 16 << 20;

/// The most widgets and log lines one frame may send the host: 65 536. The
/// byte cap alone would let a guest send millions of empty labels.
pub const MAX_FRAME_CMDS: usize = 1 << 16;

/// The soonest a guest's `request-frame` is honoured: one frame at 60 Hz. A
/// guest asking for 0 ms every frame would otherwise be the busy loop the
/// request exists to end.
pub const MIN_FRAME_MS: u32 = 16;

/// How deep groups (`ui-begin`) may nest: 32. Deeper ones are drawn flat — a
/// renderer walks groups recursively, and a guest is not trusted with the
/// depth of the host's stack.
pub const MAX_NESTING: u32 = 32;

/// What a guest may write to stderr over its life: 64 KiB. Past it the stream
/// is closed to the guest, which a runtime takes as a full pipe.
const STDERR_CAPACITY: usize = 64 << 10;

/// Start the clock that guest time limits are measured against.
///
/// Wasmtime's epoch interruption is cooperative and cheap: compiled code checks
/// a counter at loop headers and function entries, and this thread is what
/// moves the counter. Call once per engine, before any guest runs. The thread
/// holds an engine handle for the life of the process, which is the life of
/// the engine anyway.
pub fn start_epoch_ticker(engine: &Engine) {
    let engine = engine.clone();
    std::thread::Builder::new()
        .name("epoch-ticker".into())
        .spawn(move || loop {
            std::thread::sleep(EPOCH_TICK);
            engine.increment_epoch();
        })
        .expect("spawn the epoch ticker");
}

/// The memory and table limits a guest runs under, and a note of whether it
/// hit them — so a guest stopped for its appetite can be told apart from one
/// that simply crashed.
#[derive(Default)]
struct GuestLimits {
    memory_exceeded: bool,
}

impl ResourceLimiter for GuestLimits {
    /// Over the cap is a trap, not a quiet `-1` from `memory.grow`. A guest
    /// told "no" mostly aborts anyway, and the ones that do not would carry on
    /// in some half-state; stopping it here is the one outcome that is the
    /// same for every guest, whatever language it was written in.
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > MAX_GUEST_MEMORY {
            self.memory_exceeded = true;
            wasmtime::bail!("guest memory would exceed {MAX_GUEST_MEMORY} bytes");
        }
        Ok(maximum.is_none_or(|max| desired <= max))
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(maximum.is_none_or(|max| desired <= max))
    }
}

/// What the guest has sent the host since the budget last reset — at the start
/// of each frame, or at instantiation for `init` — and whether it went over.
#[derive(Default)]
struct Sent {
    cmds: usize,
    bytes: usize,
    exceeded: bool,
}

/// Per-instance host state. Reachable from every host-interface method via
/// `&mut self`, and from the store via `store.data()/data_mut()`.
pub struct HostState {
    /// UI commands the guest emitted during the current frame.
    pub ui: Vec<UiCmd>,
    /// Input for the current frame (clicked buttons, edited text).
    pub input: FrameInput,
    /// Running index handed to each `ui-button` call this frame.
    pub button_counter: u32,
    /// Running index handed to each `ui-text-edit` call this frame.
    pub text_counter: u32,
    /// Running index handed to each group (`ui-begin`) this frame.
    group_counter: u32,
    /// The index of the last text or search field drawn, for `ui-submitted`;
    /// a choice in between clears it.
    last_text_field: Option<u32>,
    /// How many groups are open right now — including any past
    /// `MAX_NESTING`, which were not drawn but still have an `ui-end` coming.
    depth: u32,
    /// Debug lines the guest sent via `log`.
    pub logs: Vec<String>,
    /// The shell's registry, so `list-apps` has something to answer with. A
    /// snapshot: the guest sees the list as it was when it was instantiated.
    apps: Vec<AppEntry>,
    /// Where the guest asked to navigate, if it did. Drained by the shell after
    /// the frame returns — see `open-app` in the WIT for why it cannot be
    /// honoured during the call.
    pending_open: Option<String>,
    /// When the guest asked for its next frame, in milliseconds, if it did.
    /// Reset each frame: a request is for the frame after this one only.
    frame_after: Option<u32>,
    /// WASI state. Mini-apps written in languages with a runtime (C#/.NET, Go,
    /// …) import the `wasi:*` interfaces even for a headless reactor, so the
    /// host must provide them; the Rust mini-apps import none, so this stays
    /// dormant for them. The context is deliberately minimal — no filesystem or
    /// network preopens, and stderr into `stderr` below rather than the
    /// shell's own.
    wasi: WasiCtx,
    /// What the guest wrote to stderr — a runtime's diagnostics — held here,
    /// bounded, and moved into `logs` (escaped) after each call. It used to be
    /// the shell's stderr, where a guest's bytes reached the terminal raw.
    stderr: MemoryOutputPipe,
    /// How much of `stderr` has been moved into `logs` already.
    stderr_seen: usize,
    table: ResourceTable,
    limits: GuestLimits,
    sent: Sent,
    /// The app's `storage-*` strings (ADR-2).
    storage: Storage,
    /// What it loaded with `image-load`.
    images: Images,
    /// Its requests, and what the user allowed (ADR-3).
    net: Net,
    /// Its element tree (ADR-4), kept between frames.
    pub dom: Document,
    /// Which of the document's limits it went past, if it did — so it is
    /// told apart from a crash, like `limits` and `sent`.
    dom_exceeded: Option<dom::Limit>,
}

/// What the shell decided about an app before starting it, from what it said
/// about itself (ADR-1) — the guest has no say in any of it.
#[derive(Default)]
pub struct Standing {
    /// The identity its storage is kept under (ADR-2): signed, and asked for
    /// `storage`. `None` is storage that ends with the app.
    pub keep_as: Option<String>,
    /// Its identity when signed, under which the user's network grants are
    /// remembered.
    pub identity: Option<String>,
    /// The origins its manifest declares under `network`.
    pub network: Vec<String>,
}

impl HostState {
    fn new(apps: Vec<AppEntry>, standing: &Standing) -> Self {
        let stderr = MemoryOutputPipe::new(STDERR_CAPACITY);
        HostState {
            ui: Vec::new(),
            input: FrameInput::default(),
            button_counter: 0,
            text_counter: 0,
            group_counter: 0,
            last_text_field: None,
            depth: 0,
            logs: Vec::new(),
            apps,
            pending_open: None,
            frame_after: None,
            wasi: WasiCtxBuilder::new().stderr(stderr.clone()).build(),
            stderr,
            stderr_seen: 0,
            table: ResourceTable::new(),
            limits: GuestLimits::default(),
            sent: Sent::default(),
            storage: Storage::open(standing.keep_as.as_deref()),
            images: Images::default(),
            net: Net::new(&standing.network, standing.identity.as_deref()),
            dom: Document::new(),
            dom_exceeded: None,
        }
    }

    /// Move what the guest wrote to stderr since last time into its logs, a
    /// line at a time, escaped like `log`.
    fn drain_stderr(&mut self) {
        let written = self.stderr.contents();
        if written.len() <= self.stderr_seen {
            return;
        }
        let new = String::from_utf8_lossy(&written[self.stderr_seen..]).into_owned();
        self.stderr_seen = written.len();
        for line in new.lines().filter(|l| !l.trim().is_empty()) {
            self.logs
                .push(format!("stderr: {}", crate::text::visible(line)));
        }
    }

    /// Charge one command of `bytes` to this frame's budget, and trap if it is
    /// spent. Called *before* the push, so the host never holds more than the
    /// cap. The string itself is already on the host by then — the canonical
    /// ABI lifts it before calling in — but it is one string, bounded by the
    /// guest's own memory, and it is dropped here rather than kept.
    fn spend(&mut self, bytes: usize) -> wasmtime::Result<()> {
        self.spend_n(1, bytes)
    }

    /// `spend` for a call that keeps `n` things — a choice's options each
    /// cost what a widget does, or a list of a million empty strings would
    /// be one command.
    fn spend_n(&mut self, n: usize, bytes: usize) -> wasmtime::Result<()> {
        self.sent.cmds += n;
        self.sent.bytes = self.sent.bytes.saturating_add(bytes);
        if self.sent.cmds > MAX_FRAME_CMDS || self.sent.bytes > MAX_FRAME_BYTES {
            self.sent.exceeded = true;
            wasmtime::bail!("guest sent more than one frame's worth of output");
        }
        Ok(())
    }
}

impl HostState {
    /// A document call's answer, or a trap if it went past the document's
    /// limits — remembered, so `explain` can say which.
    fn doc<T>(&mut self, r: Result<T, dom::Limit>) -> wasmtime::Result<T> {
        r.map_err(|limit| {
            self.dom_exceeded = Some(limit);
            wasmtime::format_err!("{}", limit.sentence())
        })
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// The capability table, now expressed as a typed trait instead of a set of
/// `(ptr, len)` shims. A mini-app can call exactly these functions and nothing
/// else crosses the sandbox — add a method here to grant a new capability.
///
/// Each method returns `wasmtime::Result`, and an `Err` is a trap in the guest.
/// Anything that keeps what the guest sends goes through `spend` first.
impl Host for HostState {
    /// Escaped on the way in: every reader of a guest's log — a terminal, the
    /// log file — is one where control characters would be commands.
    fn log(&mut self, msg: String) -> wasmtime::Result<()> {
        self.spend(msg.len())?;
        self.logs.push(crate::text::visible(&msg));
        Ok(())
    }

    fn ui_label(&mut self, text: String) -> wasmtime::Result<()> {
        self.spend(text.len())?;
        self.ui.push(UiCmd::Label(text));
        Ok(())
    }

    fn ui_button(&mut self, text: String) -> wasmtime::Result<bool> {
        self.spend(text.len())?;
        let index = self.button_counter;
        self.button_counter += 1;
        let clicked = self.input.clicked.contains(&index);
        self.ui.push(UiCmd::Button { index, text });
        Ok(clicked)
    }

    /// The guest hands in the value it holds and gets back the value after the
    /// user's edits. When the field has not been touched its own value comes
    /// straight back, so a guest that ignores the return value simply has a
    /// field that never changes rather than one that misbehaves.
    fn ui_text_edit(&mut self, text: String) -> wasmtime::Result<String> {
        let index = self.text_counter;
        self.text_counter += 1;
        let current = self.input.edits.get(&index).cloned().unwrap_or(text);
        self.spend(current.len())?;
        self.last_text_field = Some(index);
        self.ui.push(UiCmd::TextEdit {
            index,
            text: current.clone(),
        });
        Ok(current)
    }

    /// Clamped rather than rejected: the guest is untrusted, and a heading
    /// level of 9 is a typo, not an attack worth trapping over.
    fn ui_heading(&mut self, text: String, level: u8) -> wasmtime::Result<()> {
        self.spend(text.len())?;
        self.ui.push(UiCmd::Heading {
            text,
            level: level.clamp(1, 3),
        });
        Ok(())
    }

    /// Shares `text_counter` with `ui_text_edit` on purpose — a guest that
    /// swaps one for the other keeps its field indices, and so do the
    /// `--input` scripts that drive it headlessly.
    fn ui_search_field(&mut self, text: String, placeholder: String) -> wasmtime::Result<String> {
        let index = self.text_counter;
        self.text_counter += 1;
        let current = self.input.edits.get(&index).cloned().unwrap_or(text);
        self.spend(current.len() + placeholder.len())?;
        self.last_text_field = Some(index);
        self.ui.push(UiCmd::Search {
            index,
            text: current.clone(),
            placeholder,
        });
        Ok(current)
    }

    /// Shares `button_counter` with `ui_button`, for the same reason: a click
    /// is a click whatever it was drawn as.
    fn ui_tile(&mut self, title: String, subtitle: String) -> wasmtime::Result<bool> {
        self.spend(title.len() + subtitle.len())?;
        let index = self.button_counter;
        self.button_counter += 1;
        let clicked = self.input.clicked.contains(&index);
        self.ui.push(UiCmd::Tile {
            index,
            title,
            subtitle,
        });
        Ok(clicked)
    }

    fn now_millis(&mut self) -> wasmtime::Result<i64> {
        use std::time::{SystemTime, UNIX_EPOCH};
        Ok(SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0))
    }

    /// The registry, minus the start page itself.
    ///
    /// The store's entry is how the *shell* finds home; it is plumbing, not a
    /// destination, and a start page that lists itself as something to open is
    /// a card that takes you where you already are. Filtered here rather than
    /// in the store so that any catalogue written against this WIT gets it, and
    /// so no guest has to hardcode the host's name for home.
    ///
    /// `--list` still prints it: that is the registry, not the home screen, and
    /// typing `store` in the address bar goes on working.
    fn list_apps(&mut self) -> wasmtime::Result<Vec<WitAppEntry>> {
        Ok(self
            .apps
            .iter()
            .filter(|e| e.name != crate::shell::STORE_APP)
            .map(|e| WitAppEntry {
                name: e.name.clone(),
                source: e.src.clone(),
                description: e.description.clone(),
            })
            .collect())
    }

    /// Record the request; the shell acts on it after this frame. Last call
    /// wins — a guest that asks twice in one frame gets one navigation, which
    /// is the only outcome that makes sense. Kept, not accumulated, so it
    /// needs no budget.
    fn open_app(&mut self, source: String) -> wasmtime::Result<()> {
        self.pending_open = Some(source);
        Ok(())
    }

    fn storage_get(&mut self, key: String) -> wasmtime::Result<Option<String>> {
        Ok(self.storage.get(&key))
    }

    /// Charged to the frame like anything else the host keeps of a guest's.
    fn storage_set(&mut self, key: String, value: String) -> wasmtime::Result<Result<(), String>> {
        self.spend(key.len() + value.len())?;
        Ok(self.storage.set(key, value))
    }

    fn storage_remove(&mut self, key: String) -> wasmtime::Result<()> {
        self.storage.remove(&key);
        Ok(())
    }

    fn storage_keys(&mut self) -> wasmtime::Result<Vec<String>> {
        Ok(self.storage.keys())
    }

    fn ui_begin(&mut self, kind: String) -> wasmtime::Result<()> {
        self.spend(0)?;
        self.depth += 1;
        if self.depth <= MAX_NESTING {
            let index = self.group_counter;
            self.group_counter += 1;
            self.ui.push(UiCmd::Begin {
                index,
                kind: Container::from_name(&kind),
            });
        }
        Ok(())
    }

    /// An `ui-end` with no group open is ignored; one closing a group past
    /// `MAX_NESTING` closes what was never drawn.
    fn ui_end(&mut self) -> wasmtime::Result<()> {
        self.spend(0)?;
        if self.depth == 0 {
            return Ok(());
        }
        if self.depth <= MAX_NESTING {
            self.ui.push(UiCmd::End);
        }
        self.depth -= 1;
        Ok(())
    }

    /// The button index space, since it is tapped: the new value is the
    /// guest's, flipped if last frame's tap landed on it.
    fn ui_checkbox(&mut self, text: String, checked: bool) -> wasmtime::Result<bool> {
        self.spend(text.len())?;
        let index = self.button_counter;
        self.button_counter += 1;
        let checked = checked ^ self.input.clicked.contains(&index);
        self.ui.push(UiCmd::Checkbox {
            index,
            text,
            checked,
        });
        Ok(checked)
    }

    /// The text-field index space, since it holds a value: the user's pick
    /// arrives as the option's number, and anything out of range is ignored.
    fn ui_choice(&mut self, options: Vec<String>, selected: u32) -> wasmtime::Result<u32> {
        self.spend_n(options.len().max(1), options.iter().map(String::len).sum())?;
        let index = self.text_counter;
        self.text_counter += 1;
        let in_range = |i: &u32| (*i as usize) < options.len();
        let selected = self
            .input
            .edits
            .get(&index)
            .and_then(|v| v.parse::<u32>().ok())
            .filter(in_range)
            .unwrap_or(if in_range(&selected) { selected } else { 0 });
        self.last_text_field = None;
        self.ui.push(UiCmd::Choice {
            index,
            options,
            selected,
        });
        Ok(selected)
    }

    fn ui_separator(&mut self) -> wasmtime::Result<()> {
        self.spend(0)?;
        self.ui.push(UiCmd::Separator);
        Ok(())
    }

    /// Clamped, and NaN is nothing done yet: a guest's arithmetic is not
    /// worth a trap.
    fn ui_progress(&mut self, fraction: f32) -> wasmtime::Result<()> {
        self.spend(0)?;
        let fraction = if fraction.is_nan() {
            0.0
        } else {
            fraction.clamp(0.0, 1.0)
        };
        self.ui.push(UiCmd::Progress(fraction));
        Ok(())
    }

    /// About the field drawn last. After a choice, or before any field, the
    /// answer is no.
    fn ui_submitted(&mut self) -> wasmtime::Result<bool> {
        Ok(self
            .last_text_field
            .is_some_and(|i| self.input.submitted.contains(&i)))
    }

    /// The bytes are charged like any the host keeps; the decoded image is
    /// bounded by `images::MAX_APP_PIXELS` instead.
    fn image_load(&mut self, bytes: Vec<u8>) -> wasmtime::Result<Result<u32, String>> {
        self.spend(bytes.len())?;
        Ok(self.images.load(&bytes))
    }

    fn ui_image(&mut self, handle: u32) -> wasmtime::Result<()> {
        self.spend(0)?;
        if let Some(picture) = self.images.get(handle) {
            self.ui.push(UiCmd::Image(picture.clone()));
        }
        Ok(())
    }

    /// Charged like anything else the host keeps of a guest's; the policy
    /// is `Net`'s (ADR-3).
    fn fetch(&mut self, request: HttpRequest) -> wasmtime::Result<Result<u32, String>> {
        let size = request.url.len()
            + request.method.len()
            + request.body.len()
            + request
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>();
        self.spend_n(1 + request.headers.len(), size)?;
        Ok(self.net.fetch(net::Request {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body,
        }))
    }

    fn fetch_poll(
        &mut self,
        request: u32,
    ) -> wasmtime::Result<Option<Result<HttpResponse, String>>> {
        Ok(self.net.poll(request).map(|answer| {
            answer.map(|r| HttpResponse {
                status: r.status,
                headers: r.headers,
                body: r.body,
            })
        }))
    }

    fn locale(&mut self) -> wasmtime::Result<String> {
        Ok(crate::locale())
    }

    /// The earliest request in a frame wins, and none is sooner than
    /// `MIN_FRAME_MS`. Kept, not accumulated, like `open_app`.
    fn request_frame(&mut self, after_ms: u32) -> wasmtime::Result<()> {
        let ms = after_ms.max(MIN_FRAME_MS);
        self.frame_after = Some(self.frame_after.map_or(ms, |m| m.min(ms)));
        Ok(())
    }
}

/// The element tree (ADR-4). Every edit is checked against the document's
/// limits rather than the frame's: the tree outlives the frame, so it is the
/// tree that has to be bounded. A number that is not a node is ignored.
impl DomHost for HostState {
    fn root(&mut self) -> wasmtime::Result<u32> {
        Ok(dom::ROOT)
    }

    fn create_element(&mut self, tag: String) -> wasmtime::Result<u32> {
        let r = self.dom.create_element(&tag);
        self.doc(r)
    }

    fn create_text(&mut self, text: String) -> wasmtime::Result<u32> {
        let r = self.dom.create_text(&text);
        self.doc(r)
    }

    fn append_child(&mut self, parent: u32, child: u32) -> wasmtime::Result<()> {
        let r = self.dom.append_child(parent, child);
        self.doc(r)
    }

    fn insert_before(&mut self, parent: u32, child: u32, before: u32) -> wasmtime::Result<()> {
        let r = self.dom.insert_before(parent, child, Some(before));
        self.doc(r)
    }

    fn remove(&mut self, child: u32) -> wasmtime::Result<()> {
        self.dom.remove(child);
        Ok(())
    }

    fn release(&mut self, item: u32) -> wasmtime::Result<()> {
        self.dom.release(item);
        Ok(())
    }

    fn set_text(&mut self, item: u32, text: String) -> wasmtime::Result<()> {
        let r = self.dom.set_text(item, &text);
        self.doc(r)
    }

    fn set_attribute(&mut self, item: u32, name: String, value: String) -> wasmtime::Result<()> {
        let r = self.dom.set_attribute(item, &name, &value);
        self.doc(r)
    }

    fn remove_attribute(&mut self, item: u32, name: String) -> wasmtime::Result<()> {
        let r = self.dom.remove_attribute(item, &name);
        self.doc(r)
    }

    fn get_attribute(&mut self, item: u32, name: String) -> wasmtime::Result<Option<String>> {
        Ok(self.dom.get_attribute(item, &name))
    }

    fn toggle_class(&mut self, item: u32, name: String, on: bool) -> wasmtime::Result<()> {
        let r = self.dom.toggle_class(item, &name, on);
        self.doc(r)
    }

    fn set_style(&mut self, item: u32, property: String, value: String) -> wasmtime::Result<()> {
        let r = self.dom.set_style(item, &property, &value);
        self.doc(r)
    }

    fn set_markup(&mut self, _item: u32, _markup: String) -> wasmtime::Result<Result<(), String>> {
        Ok(Err("set-markup is not in this build yet".into()))
    }

    fn query_all(&mut self, scope: u32, selector: String) -> wasmtime::Result<Vec<u32>> {
        Ok(self.dom.query_all(scope, &selector))
    }

    fn parent(&mut self, item: u32) -> wasmtime::Result<Option<u32>> {
        Ok(self.dom.parent(item))
    }

    fn children(&mut self, item: u32) -> wasmtime::Result<Vec<u32>> {
        Ok(self.dom.children(item))
    }

    fn value(&mut self, item: u32) -> wasmtime::Result<String> {
        Ok(self.dom.value(item))
    }

    fn focus(&mut self, item: u32) -> wasmtime::Result<()> {
        self.dom.focus(item);
        Ok(())
    }

    /// From the last layout, which the frame before this one drew.
    fn bounds(&mut self, item: u32) -> wasmtime::Result<Option<Rect>> {
        Ok(self
            .dom
            .last_layout()
            .and_then(|l| l.rect_of(item))
            .map(|r| Rect {
                x: r.x,
                y: r.y,
                width: r.w,
                height: r.h,
            }))
    }

    /// What could not be read goes to the app's log, as a browser's
    /// console would have it.
    fn add_stylesheet(&mut self, css: String) -> wasmtime::Result<u32> {
        let r = self.dom.add_stylesheet(&css);
        let (id, warnings) = self.doc(r)?;
        for w in warnings {
            self.logs.push(format!("css: {}", crate::text::visible(&w)));
        }
        Ok(id)
    }

    fn remove_stylesheet(&mut self, sheet: u32) -> wasmtime::Result<()> {
        self.dom.remove_stylesheet(sheet);
        Ok(())
    }

    fn listen(&mut self, item: u32, kind: String) -> wasmtime::Result<()> {
        let r = self.dom.listen(item, &kind);
        self.doc(r)
    }

    fn unlisten(&mut self, item: u32, kind: String) -> wasmtime::Result<()> {
        let r = self.dom.unlisten(item, &kind);
        self.doc(r)
    }

    fn events(&mut self) -> wasmtime::Result<Vec<WitEvent>> {
        Ok(self
            .dom
            .take_events()
            .into_iter()
            .map(|e| WitEvent {
                kind: e.kind,
                target: e.target,
                current: e.current,
                value: e.value,
                x: e.x,
                y: e.y,
            })
            .collect())
    }
}

/// A loaded, instantiated mini-app component plus its per-frame entry point.
/// Same public surface as the old core-module version.
pub struct MiniApp {
    store: Store<HostState>,
    world: MiniAppWorld,
}

impl MiniApp {
    /// Compile + instantiate a `.wasm` **component** mini-app and run its
    /// `init`. `apps` is the registry snapshot the guest may read back through
    /// `list-apps` — the store's whole reason for existing.
    ///
    /// Compilation is Cranelift turning a whole component into machine code —
    /// milliseconds for a 19 KB Rust mini-app, but the C# one carries a trimmed
    /// .NET runtime and is over two megabytes. That is why `MiniApp` is built
    /// off the UI thread (see `Shell::start`) rather than inside a frame: every
    /// type this touches is `Send`, so the finished app can simply be handed
    /// over when it is ready.
    ///
    /// `standing` is what the shell decided about the app from its manifest
    /// and signature: where its storage is kept, what network it declared.
    pub fn load(
        engine: &Engine,
        wasm: &[u8],
        apps: Vec<AppEntry>,
        standing: &Standing,
    ) -> Result<Self> {
        let component = Component::from_binary(engine, wasm)?;
        if let Some(missing) = newer_than_us(engine, &component) {
            return Err(anyhow!(
                "this app needs a newer nimadir: it uses {missing}, which nimadir {} \
                 (WIT {}) does not have",
                env!("CARGO_PKG_VERSION"),
                crate::wit_package()
            ));
        }

        let mut linker: Linker<HostState> = Linker::new(engine);
        // `HasSelf<HostState>` says "the host data IS the store data" — the getter
        // is the identity `|s| s`. (wasmtime 47's typed-linker mechanism.)
        MiniAppWorld::add_to_linker::<HostState, HasSelf<HostState>>(
            &mut linker,
            |s: &mut HostState| s,
        )?;

        // WASI 0.2, so components from runtime-bearing languages (C#/.NET, Go)
        // resolve their `wasi:*` imports (poll, clocks, random, …). Additive —
        // the Rust mini-apps import none of it.
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

        let mut store = Store::new(engine, HostState::new(apps, standing));
        store.limiter(|s| &mut s.limits);
        // Set before instantiation, not just before `init`: a component can run
        // code while it is being instantiated, and with epoch interruption on,
        // a store with no deadline set traps at the first check.
        store.set_epoch_deadline(INIT_BUDGET_TICKS);

        let started = MiniAppWorld::instantiate(&mut store, &component, &linker)
            .and_then(|world| world.call_init(&mut store).map(|()| world));
        store.data_mut().drain_stderr();
        let world = started.map_err(|e| explain(e, &store, "start", INIT_BUDGET_TICKS))?;

        Ok(MiniApp { store, world })
    }

    /// Run exactly one frame: feed this frame's input in, let the guest draw,
    /// return the UI it described. This is the whole host<->guest cycle.
    pub fn frame(&mut self, input: FrameInput) -> Result<Vec<UiCmd>> {
        {
            let st = self.store.data_mut();
            st.ui.clear();
            st.button_counter = 0;
            st.text_counter = 0;
            st.group_counter = 0;
            st.last_text_field = None;
            st.depth = 0;
            st.sent = Sent::default();
            st.frame_after = None;
            st.input = input;
        }
        self.store.set_epoch_deadline(FRAME_BUDGET_TICKS);
        let updated = self.world.call_update(&mut self.store);
        self.store.data_mut().drain_stderr();
        updated.map_err(|e| explain(e, &self.store, "finish a frame", FRAME_BUDGET_TICKS))?;
        let st = self.store.data_mut();
        // Close what the guest left open, so a backend always sees groups
        // balanced.
        for _ in 0..st.depth.min(MAX_NESTING) {
            st.ui.push(UiCmd::End);
        }
        // Requests out on the network are polled from frames; there have to
        // be some.
        if st.net.flying() {
            let ms = 50;
            st.frame_after = Some(st.frame_after.map_or(ms, |m| m.min(ms)));
        }
        // Storage written now, or a frame asked for when it is due.
        if let Some(ms) = st.storage.tick() {
            let ms = ms.max(MIN_FRAME_MS);
            st.frame_after = Some(st.frame_after.map_or(ms, |m| m.min(ms)));
        }
        Ok(self.store.data().ui.clone())
    }

    /// Drain any log lines the guest emitted since the last call.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().logs)
    }

    /// Take the navigation the guest asked for, if it asked for one. Called
    /// after `frame`, when the guest is no longer on the stack and dropping it
    /// is legal.
    pub fn take_pending_open(&mut self) -> Option<String> {
        let st = self.store.data_mut();
        st.pending_open
            .take()
            .or_else(|| st.dom.take_pending_open())
    }

    /// The app's element tree, for a renderer to draw and the user to act on.
    pub fn document(&self) -> &Document {
        &self.store.data().dom
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.store.data_mut().dom
    }

    /// The app's page laid out on `env`, with its images sized from what it
    /// loaded.
    pub fn layout_page(
        &mut self,
        env: &dom::css::Env,
        fonts: &dyn dom::layout::Fonts,
    ) -> &dom::layout::Layout {
        let st = self.store.data_mut();
        let images = &st.images;
        let size = |h: u32| images.get(h).map(|p| (p.width as f32, p.height as f32));
        st.dom.layout(env, fonts, &size)
    }

    /// An image the app loaded, by its number.
    pub fn image(&self, handle: u32) -> Option<crate::images::Picture> {
        self.store.data().images.get(handle).cloned()
    }

    /// Replace the registry `list-apps` answers with — a fresher one arrived
    /// while this app was running.
    pub fn set_apps(&mut self, apps: Vec<AppEntry>) {
        self.store.data_mut().apps = apps;
    }

    /// The origin the user is being asked whether the app may reach.
    pub fn network_question(&self) -> Option<String> {
        self.store.data().net.asking().map(str::to_string)
    }

    pub fn answer_network(&mut self, allow: bool) {
        self.store.data_mut().net.answer(allow);
    }

    /// Wait for the app's requests under way — the headless runner's way of
    /// keeping a run deterministic.
    pub fn settle_network(&mut self) {
        self.store.data_mut().net.settle(Duration::from_secs(30));
    }

    /// When the guest wants its next frame, in milliseconds, if it asked.
    pub fn take_frame_request(&mut self) -> Option<u32> {
        self.store.data_mut().frame_after.take()
    }
}

/// What a component asks of `nimadir:shell` that this host does not provide,
/// named for a person — `None` when everything it asks for is here.
///
/// Checked before linking, because the linker's own account of it ("a matching
/// implementation was not found … the wrong type: function implementation is
/// missing") is true and tells nobody what to do. The WIT is additive-only and
/// each addition bumps its minor version, so an import this host lacks is one
/// added after it was built: the app is newer than the shell. Other imports
/// (`wasi:*`) are left to the linker.
fn newer_than_us(engine: &Engine, component: &Component) -> Option<String> {
    use wasmtime::component::types::ComponentItem;

    let package = crate::wit_package();
    let (name, version) = package.split_once('@').unwrap_or((package, ""));
    let major = |v: &str| v.split('.').next().unwrap_or("").to_string();
    let provided = provided_functions();

    let mut missing = Vec::new();
    let ty = component.component_type();
    for (import, item) in ty.imports(engine) {
        let Some(rest) = import.strip_prefix(&format!("{name}/")) else {
            continue;
        };
        let (interface, theirs) = rest.split_once('@').unwrap_or((rest, ""));
        // Another interface, or another major version, is nothing this host
        // has. Within a major version wasmtime links across minor ones
        // (semver), so there the functions themselves decide.
        let known = provided.iter().any(|(i, _)| *i == interface);
        if !known || major(theirs) != major(version) {
            missing.push(format!("`{import}`"));
            continue;
        }
        if let ComponentItem::ComponentInstance(instance) = item.ty {
            // Functions only: an interface exports its types too (`app-entry`),
            // and a type is not something a host provides or lacks.
            for (function, export) in instance.exports(engine) {
                if matches!(export.ty, ComponentItem::ComponentFunc(_))
                    && !provided.contains(&(interface, function))
                {
                    missing.push(format!("`{function}`"));
                }
            }
        }
    }
    (!missing.is_empty()).then(|| missing.join(", "))
}

/// The functions each interface declares, as `(interface, function)`, read
/// from the same WIT file `bindgen!` compiled — so this list cannot drift from
/// what is implemented.
fn provided_functions() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    let mut interface = None;
    for line in include_str!("../../wit/world.wit").lines() {
        if let Some(name) = line.strip_prefix("interface ") {
            interface = name.split_whitespace().next();
            continue;
        }
        if line.starts_with('}') {
            interface = None;
            continue;
        }
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        if let (Some(i), Some((name, _))) = (interface, line.split_once(": func(")) {
            out.push((i, name));
        }
    }
    out
}

/// Turn a guest failure into something worth putting in front of a person.
///
/// The limits this module enforces get a sentence that says which limit it
/// was; anything else is a genuine crash, and keeps the whole chain — the
/// trap's own wording is the only clue to *what* the guest did wrong.
fn explain(e: wasmtime::Error, store: &Store<HostState>, doing: &str, ticks: u64) -> anyhow::Error {
    if store.data().limits.memory_exceeded {
        return anyhow!(
            "it tried to use more than {} MiB of memory and was stopped",
            MAX_GUEST_MEMORY >> 20
        );
    }
    if let Some(limit) = store.data().dom_exceeded {
        return anyhow!("it {} and was stopped", limit.sentence());
    }
    if store.data().sent.exceeded {
        return anyhow!(
            "it sent more than {MAX_FRAME_CMDS} widgets or {} MiB of text in one frame and was stopped",
            MAX_FRAME_BYTES >> 20
        );
    }
    if e.downcast_ref::<Trap>() == Some(&Trap::Interrupt) {
        let budget = EPOCH_TICK * ticks as u32;
        return anyhow!("it did not {doing} within {:.0?} and was stopped", budget);
    }
    anyhow!("{e:#}")
}
