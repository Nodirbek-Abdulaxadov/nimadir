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
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::registry::AppEntry;
use crate::storage::Storage;
use crate::ui::{FrameInput, UiCmd};

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

use bindings::nimadir::shell::host_api::{AppEntry as WitAppEntry, Host};
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
    /// network preopens — a sandboxed default (only stderr is inherited, for
    /// runtime diagnostics).
    wasi: WasiCtx,
    table: ResourceTable,
    limits: GuestLimits,
    sent: Sent,
    /// The app's `storage-*` strings (ADR-2).
    storage: Storage,
}

impl HostState {
    fn new(apps: Vec<AppEntry>, storage: Storage) -> Self {
        HostState {
            ui: Vec::new(),
            input: FrameInput::default(),
            button_counter: 0,
            text_counter: 0,
            logs: Vec::new(),
            apps,
            pending_open: None,
            frame_after: None,
            wasi: WasiCtxBuilder::new().inherit_stderr().build(),
            table: ResourceTable::new(),
            limits: GuestLimits::default(),
            sent: Sent::default(),
            storage,
        }
    }

    /// Charge one command of `bytes` to this frame's budget, and trap if it is
    /// spent. Called *before* the push, so the host never holds more than the
    /// cap. The string itself is already on the host by then — the canonical
    /// ABI lifts it before calling in — but it is one string, bounded by the
    /// guest's own memory, and it is dropped here rather than kept.
    fn spend(&mut self, bytes: usize) -> wasmtime::Result<()> {
        self.sent.cmds += 1;
        self.sent.bytes = self.sent.bytes.saturating_add(bytes);
        if self.sent.cmds > MAX_FRAME_CMDS || self.sent.bytes > MAX_FRAME_BYTES {
            self.sent.exceeded = true;
            wasmtime::bail!("guest sent more than one frame's worth of output");
        }
        Ok(())
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
    fn log(&mut self, msg: String) -> wasmtime::Result<()> {
        self.spend(msg.len())?;
        self.logs.push(msg);
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

    /// The earliest request in a frame wins, and none is sooner than
    /// `MIN_FRAME_MS`. Kept, not accumulated, like `open_app`.
    fn request_frame(&mut self, after_ms: u32) -> wasmtime::Result<()> {
        let ms = after_ms.max(MIN_FRAME_MS);
        self.frame_after = Some(self.frame_after.map_or(ms, |m| m.min(ms)));
        Ok(())
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
    /// `keep_as` is the identity whose storage the app gets (ADR-2): `None`
    /// gives it storage that ends with it.
    pub fn load(
        engine: &Engine,
        wasm: &[u8],
        apps: Vec<AppEntry>,
        keep_as: Option<&str>,
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

        let mut store = Store::new(engine, HostState::new(apps, Storage::open(keep_as)));
        store.limiter(|s| &mut s.limits);
        // Set before instantiation, not just before `init`: a component can run
        // code while it is being instantiated, and with epoch interruption on,
        // a store with no deadline set traps at the first check.
        store.set_epoch_deadline(INIT_BUDGET_TICKS);

        let started = MiniAppWorld::instantiate(&mut store, &component, &linker)
            .and_then(|world| world.call_init(&mut store).map(|()| world));
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
            st.sent = Sent::default();
            st.frame_after = None;
            st.input = input;
        }
        self.store.set_epoch_deadline(FRAME_BUDGET_TICKS);
        self.world
            .call_update(&mut self.store)
            .map_err(|e| explain(e, &self.store, "finish a frame", FRAME_BUDGET_TICKS))?;
        // Storage written now, or a frame asked for when it is due.
        let st = self.store.data_mut();
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
        self.store.data_mut().pending_open.take()
    }

    /// Replace the registry `list-apps` answers with — a fresher one arrived
    /// while this app was running.
    pub fn set_apps(&mut self, apps: Vec<AppEntry>) {
        self.store.data_mut().apps = apps;
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
/// missing") is true and tells nobody what to do. The WIT is additive-only, so
/// an import this host lacks is one added after it was built: the app is newer
/// than the shell. Other imports (`wasi:*`) are left to the linker.
fn newer_than_us(engine: &Engine, component: &Component) -> Option<String> {
    use wasmtime::component::types::ComponentItem;

    let package = crate::wit_package();
    let (name, version) = package.split_once('@').unwrap_or((package, ""));
    let ours = format!("{name}/host-api@{version}");
    let provided = provided_functions();

    let mut missing = Vec::new();
    let ty = component.component_type();
    for (import, item) in ty.imports(engine) {
        if !import.starts_with(&format!("{name}/")) {
            continue;
        }
        if import != ours {
            missing.push(format!("`{import}`"));
            continue;
        }
        if let ComponentItem::ComponentInstance(instance) = item.ty {
            // Functions only: an interface exports its types too (`app-entry`),
            // and a type is not something a host provides or lacks.
            for (function, export) in instance.exports(engine) {
                if matches!(export.ty, ComponentItem::ComponentFunc(_))
                    && !provided.contains(&function)
                {
                    missing.push(format!("`{function}`"));
                }
            }
        }
    }
    (!missing.is_empty()).then(|| missing.join(", "))
}

/// The functions `interface host-api` declares, read from the same WIT file
/// `bindgen!` compiled — so this list cannot drift from what is implemented.
fn provided_functions() -> Vec<&'static str> {
    include_str!("../../wit/world.wit")
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("export ") && !l.starts_with("//"))
        .filter_map(|l| l.split_once(": func(").map(|(name, _)| name))
        .collect()
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
