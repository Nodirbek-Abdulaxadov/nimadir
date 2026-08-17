//! The WASM Component-Model boundary: embeds Wasmtime, implements the typed host
//! interface generated from `wit/`, and drives one guest frame at a time.
//!
//! The mini-app is now a **component**, and the host<->guest contract is the
//! `wit/` package — not a hand-rolled `(ptr, len)` ABI. `bindgen!` turns that
//! WIT into the `Host` trait we implement below (the capability table) and the
//! `MiniAppWorld` we instantiate. The canonical ABI moves strings/bools across
//! the boundary, so there is no manual linear-memory reading here any more.
//!
//! The public API of `MiniApp` (`load` / `frame` / `take_logs`) is unchanged, so
//! `main.rs` and `gui.rs` are untouched by this migration — only the internals.

use anyhow::Result;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::registry::AppEntry;
use crate::ui::{FrameInput, UiCmd};

/// Generated bindings for the `mini-app` world, kept in a submodule so the
/// generated world type (`bindings::MiniApp`) doesn't clash with our own
/// `MiniApp` wrapper below.
mod bindings {
    wasmtime::component::bindgen!({
        world: "mini-app",
        path: "../wit",
    });
}

use bindings::nimadir::shell::host_api::{AppEntry as WitAppEntry, Host};
use bindings::MiniApp as MiniAppWorld;

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
    /// WASI state. Mini-apps written in languages with a runtime (C#/.NET, Go,
    /// …) import the `wasi:*` interfaces even for a headless reactor, so the
    /// host must provide them; the Rust mini-apps import none, so this stays
    /// dormant for them. The context is deliberately minimal — no filesystem or
    /// network preopens — a sandboxed default (only stderr is inherited, for
    /// runtime diagnostics).
    wasi: WasiCtx,
    table: ResourceTable,
}

impl HostState {
    fn new(apps: Vec<AppEntry>) -> Self {
        HostState {
            ui: Vec::new(),
            input: FrameInput::default(),
            button_counter: 0,
            text_counter: 0,
            logs: Vec::new(),
            apps,
            pending_open: None,
            wasi: WasiCtxBuilder::new().inherit_stderr().build(),
            table: ResourceTable::new(),
        }
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
impl Host for HostState {
    fn log(&mut self, msg: String) {
        self.logs.push(msg);
    }

    fn ui_label(&mut self, text: String) {
        self.ui.push(UiCmd::Label(text));
    }

    fn ui_button(&mut self, text: String) -> bool {
        let index = self.button_counter;
        self.button_counter += 1;
        let clicked = self.input.clicked.contains(&index);
        self.ui.push(UiCmd::Button { index, text });
        clicked
    }

    /// The guest hands in the value it holds and gets back the value after the
    /// user's edits. When the field has not been touched its own value comes
    /// straight back, so a guest that ignores the return value simply has a
    /// field that never changes rather than one that misbehaves.
    fn ui_text_edit(&mut self, text: String) -> String {
        let index = self.text_counter;
        self.text_counter += 1;
        let current = self.input.edits.get(&index).cloned().unwrap_or(text);
        self.ui.push(UiCmd::TextEdit {
            index,
            text: current.clone(),
        });
        current
    }

    fn now_millis(&mut self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn list_apps(&mut self) -> Vec<WitAppEntry> {
        self.apps
            .iter()
            .map(|e| WitAppEntry {
                name: e.name.clone(),
                source: e.src.clone(),
                description: e.description.clone(),
            })
            .collect()
    }

    /// Record the request; the shell acts on it after this frame. Last call
    /// wins — a guest that asks twice in one frame gets one navigation, which
    /// is the only outcome that makes sense.
    fn open_app(&mut self, source: String) {
        self.pending_open = Some(source);
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
    pub fn load(engine: &Engine, wasm: &[u8], apps: Vec<AppEntry>) -> Result<Self> {
        let component = Component::from_binary(engine, wasm)?;

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

        let mut store = Store::new(engine, HostState::new(apps));
        let world = MiniAppWorld::instantiate(&mut store, &component, &linker)?;

        // One-time setup hook.
        world.call_init(&mut store)?;

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
            st.input = input;
        }
        self.world.call_update(&mut self.store)?;
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
}
