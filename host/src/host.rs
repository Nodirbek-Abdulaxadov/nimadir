//! The WASM runtime boundary: embeds Wasmtime, defines the host-function table
//! (the mini-app's "syscalls"), and drives one guest frame at a time.
//!
//! Everything a mini-app is allowed to do is exactly the set of functions
//! registered in `register_host_fns`. That IS the capability model — a mini-app
//! is sandboxed by Wasmtime and can only reach the host through this table.

use std::collections::HashSet;
use wasmtime::*;

use crate::ui::{FrameInput, UiCmd};

/// Per-instance host state, stored inside the Wasmtime `Store` and reachable
/// from every host function via `caller.data()/data_mut()`.
pub struct HostState {
    /// The guest's exported linear memory (set right after instantiation).
    pub memory: Option<Memory>,
    /// UI commands the guest emitted during the current frame.
    pub ui: Vec<UiCmd>,
    /// Input for the current frame (clicked button indices).
    pub input: FrameInput,
    /// Running index handed to each `ui_button` call this frame.
    pub button_counter: u32,
    /// Debug lines the guest sent via `host_log`.
    pub logs: Vec<String>,
}

impl HostState {
    fn new() -> Self {
        HostState {
            memory: None,
            ui: Vec::new(),
            input: FrameInput::default(),
            button_counter: 0,
            logs: Vec::new(),
        }
    }
}

/// A loaded, instantiated mini-app plus its render loop entry point.
pub struct MiniApp {
    store: Store<HostState>,
    update: TypedFunc<(), ()>,
}

impl MiniApp {
    /// Compile + instantiate a `.wasm` mini-app and run its `init()`.
    pub fn load(engine: &Engine, wasm: &[u8]) -> anyhow::Result<Self> {
        let module = Module::from_binary(engine, wasm)?;

        let mut linker: Linker<HostState> = Linker::new(engine);
        register_host_fns(&mut linker)?;

        let mut store = Store::new(engine, HostState::new());
        let instance = linker.instantiate(&mut store, &module)?;

        // Cache the guest's linear memory so host functions can read strings.
        let memory = instance.get_memory(&mut store, "memory");
        store.data_mut().memory = memory;

        // Optional one-time setup hook.
        if let Ok(init) = instance.get_typed_func::<(), ()>(&mut store, "init") {
            init.call(&mut store, ())?;
        }

        let update = instance
            .get_typed_func::<(), ()>(&mut store, "update")
            .map_err(|_| anyhow::anyhow!("mini-app has no `update()` export"))?;

        Ok(MiniApp { store, update })
    }

    /// Run exactly one frame: feed this frame's clicks in, let the guest draw,
    /// return the UI it described. This is the whole host<->guest cycle.
    pub fn frame(&mut self, clicks: HashSet<u32>) -> anyhow::Result<Vec<UiCmd>> {
        {
            let st = self.store.data_mut();
            st.ui.clear();
            st.button_counter = 0;
            st.input.clicked = clicks;
        }
        self.update.call(&mut self.store, ())?;
        Ok(self.store.data().ui.clone())
    }

    /// Drain any log lines the guest emitted since the last call.
    pub fn take_logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().logs)
    }
}

/// Register the complete capability table the guest may import from `"env"`.
/// Add a capability here and it becomes callable by every mini-app; remove it
/// and no mini-app can reach it. Nothing else crosses the sandbox boundary.
fn register_host_fns(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    // host_log(ptr, len) — debug logging.
    linker.func_wrap(
        "env",
        "host_log",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
            let bytes = read_guest_bytes(&caller, ptr, len);
            let s = String::from_utf8_lossy(&bytes).into_owned();
            caller.data_mut().logs.push(s);
        },
    )?;

    // ui_label(ptr, len) — draw a text label.
    linker.func_wrap(
        "env",
        "ui_label",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
            let bytes = read_guest_bytes(&caller, ptr, len);
            let text = String::from_utf8_lossy(&bytes).into_owned();
            caller.data_mut().ui.push(UiCmd::Label(text));
        },
    )?;

    // ui_button(ptr, len) -> i32 — draw a button; 1 if it was clicked this frame.
    linker.func_wrap(
        "env",
        "ui_button",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
            let bytes = read_guest_bytes(&caller, ptr, len);
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let st = caller.data_mut();
            let index = st.button_counter;
            st.button_counter += 1;
            let clicked = st.input.clicked.contains(&index);
            st.ui.push(UiCmd::Button { index, text });
            if clicked {
                1
            } else {
                0
            }
        },
    )?;

    // host_now_millis() -> i64 — an example capability the host fully controls.
    linker.func_wrap(
        "env",
        "host_now_millis",
        |_caller: Caller<'_, HostState>| -> i64 {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        },
    )?;

    Ok(())
}

/// Copy `len` bytes at `ptr` out of the guest's linear memory. Copies eagerly
/// (returns an owned `Vec`) so the caller can then mutate `HostState` — a later
/// guest call can grow/move memory, so we must not hold a borrow into it.
fn read_guest_bytes(caller: &Caller<'_, HostState>, ptr: i32, len: i32) -> Vec<u8> {
    let mem = match caller.data().memory {
        Some(m) => m,
        None => return Vec::new(),
    };
    let data = mem.data(caller);
    let start = ptr as usize;
    let end = start.saturating_add(len as usize);
    if len < 0 || end > data.len() {
        return Vec::new();
    }
    data[start..end].to_vec()
}
