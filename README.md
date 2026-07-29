# nimadir — a native app shell that runs sandboxed WASM mini-apps

An MVP of a "super-app / application-OS" idea: a **native** desktop shell that
loads **WebAssembly mini-apps on demand** and renders their UI natively, with
**zero web stack** — no HTML, no CSS, no JavaScript, no webview.

It behaves *like* a browser (fetch remote code, run it sandboxed, expose
capabilities) **without being one**. Third-party mini-apps are WASM modules
fetched at runtime that talk to the host through a small, typed, capability-scoped
ABI. The host is a generic renderer + capability provider; each mini-app owns its
own logic and state inside a Wasmtime sandbox.

Everything is Rust: the host embeds **Wasmtime** (WASM runtime) and, for the
window, **egui** (pure-Rust, GPU-rendered, no DOM). No FFI between them — two
crates in one binary.

```
┌──────────────────────────────────────────────────────────┐
│  NATIVE RUST HOST (the shell)                              │
│  window + GUI (egui) + Wasmtime runtime + capability table │
│                                                            │
│    ┌───────────────── Wasmtime sandbox ───────────────┐   │
│    │  mini-app.wasm  (untrusted, fetched at runtime)   │   │
│    │  exports: init(), update(), guest_alloc()         │   │
│    │  imports: host_log, ui_label, ui_button, …        │   │
│    └───────────────────────────────────────────────────┘   │
│              ▲  native host<->guest boundary  ▲            │
│              │  (NO JavaScript, NO DOM)        │            │
└──────────────────────────────────────────────────────────┘
                     NATIVE OS (Win / macOS / Linux)
```

## Quick start

```bash
# 1. one-time: the WASM target for building mini-apps
rustup target add wasm32-unknown-unknown

# 2. build a mini-app -> a .wasm file
cargo build --release --target wasm32-unknown-unknown \
  --manifest-path mini-apps/counter/Cargo.toml

# 3a. open the SHELL (a native window; starts on the home screen)
cargo run -p host --features gui

# 3b. or go straight to one app, skipping the home screen
cargo run -p host --features gui -- counter

# 3c. run HEADLESS (default; builds & runs anywhere, no system GUI libs)
cargo run -p host -- counter --script "1:0,2:0,3:0,5:1" --frames 8
```

The host also loads a mini-app straight from a URL — the "browser-like" part:

```bash
cargo run -p host -- https://example.com/some-mini-app.wasm
```

Point it at a different `.wasm` (file **or** URL) and a different mini-app runs —
**with no host rebuild.**

## The home screen

Opened with no argument, the shell behaves like a browser start page: the
mini-apps listed in `apps.list` as links, plus an address bar for any path or
URL. Clicking a link fetches that `.wasm`, instantiates it, and runs it in the
same window; **Home** drops it and goes back.

```
apps.list           # name | source (path or URL) | description
counter | mini-apps/counter/target/wasm32-unknown-unknown/release/counter.wasm | A counter…
hello   | mini-apps/hello/target/wasm32-unknown-unknown/release/hello.wasm     | A second app…
```

`cargo run -p host -- --list` prints the same list as text (there is no home
screen to click headlessly). A name from the list works anywhere a path does, so
`-- counter` and `-- mini-apps/counter/…/counter.wasm` are the same request.

Two properties fall out of the design and are worth stating:

- **Chrome and page are separate.** The home screen, address bar, and Home
  button are host UI; the guest only ever paints into the page area below them.
  Navigation is never reachable by untrusted code.
- **Leaving an app destroys it.** Navigating home drops the `MiniApp`, its
  Wasmtime `Store`, and the guest's linear memory. Reopening `counter` starts at
  0 again — the sandbox doing its job, not lost state.

A link that fails to load (missing file, dead URL, invalid module) leaves the
shell on the home screen with the error shown in red. A bad link cannot take the
shell down.

## The ABI (host <-> mini-app boundary)

This is the entire contract. It lives in one place conceptually: the host side is
`host/src/host.rs` (`register_host_fns`), the guest side is each mini-app's
`extern "C"` block. Strings are passed as `(ptr, len)` into the guest's linear
memory; the host reads them out by copy.

**Host functions** (imports the guest may call — the capability table). A mini-app
can do *only* what is listed here; nothing else crosses the sandbox:

| Function | Signature | Meaning |
|---|---|---|
| `host_log` | `(ptr: i32, len: i32)` | Debug log to host stdout. |
| `ui_label` | `(ptr: i32, len: i32)` | Draw a text label this frame. |
| `ui_button` | `(ptr: i32, len: i32) -> i32` | Draw a button; returns `1` if it was clicked this frame, else `0`. |
| `host_now_millis` | `() -> i64` | Example capability the host controls (wall-clock ms). |

**Guest exports** (the host calls these):

| Export | Signature | Meaning |
|---|---|---|
| `init` | `()` | One-time setup (optional). |
| `update` | `()` | Called once per frame; the guest re-describes its whole UI. |
| `guest_alloc` | `(len: i32) -> i32` | Lets the host write `len` bytes into guest memory (for host→guest data). |

The UI is **immediate mode**: every frame the host calls `update()`, the guest
calls `ui_label`/`ui_button` to describe what to show, and the host renders it and
feeds click results back through `ui_button`'s return value.

## How to write your own mini-app

A mini-app is a `cdylib` crate targeting `wasm32-unknown-unknown`:

```rust
#[link(wasm_import_module = "env")]
extern "C" {
    fn ui_label(ptr: i32, len: i32);
    fn ui_button(ptr: i32, len: i32) -> i32;
}
fn label(s: &str)  { unsafe { ui_label(s.as_ptr() as i32, s.len() as i32) } }
fn button(s: &str) -> bool { unsafe { ui_button(s.as_ptr() as i32, s.len() as i32) != 0 } }

#[no_mangle] pub extern "C" fn init() {}
#[no_mangle] pub extern "C" fn update() {
    label("my mini-app");
    if button("click me") { label("clicked!"); }
}
#[no_mangle] pub extern "C" fn guest_alloc(len: i32) -> i32 {
    let mut b = Vec::<u8>::with_capacity(len.max(0) as usize);
    let p = b.as_mut_ptr() as i32; std::mem::forget(b); p
}
```

See `mini-apps/counter` (guest-owned state) and `mini-apps/hello` (a second app)
for complete examples.

## Layout

```
Cargo.toml                 # workspace = [host]; mini-apps are excluded (wasm target)
apps.list                  # the home screen's links ("bookmarks")
host/
  src/main.rs              # CLI, arg parsing, headless loop
  src/host.rs              # Wasmtime embedding, HostState, capability table, one-frame driver
  src/ui.rs                # UiCmd / FrameInput — the renderer-agnostic UI protocol
  src/registry.rs          # apps.list parsing; name -> source resolution
  src/shell.rs             # navigation (Home <-> App), fetch-from-file/URL
  src/gui.rs               # native egui window backend (feature "gui")
mini-apps/
  counter/                 # sample: a counter; state lives inside the guest
  hello/                   # sample: a second app, to show hot-swap without rebuild
```

## MVP status — all four milestones verified

- **M1 — native shell.** Host builds and runs; egui window backend builds (feature `gui`).
- **M2 — boundary, both directions.** Host calls guest `init`/`update`; guest calls
  `host_log` / `ui_label` / `ui_button` / `host_now_millis`, with the host reading
  strings out of guest linear memory. No JS anywhere.
- **M3 — immediate-mode UI + a real mini-app.** The `counter` mini-app draws a label
  and Increment/Reset buttons; the count lives **inside the WASM guest**, the host
  just renders and routes clicks.
- **M4 — on-demand loading.** The same host binary runs `counter.wasm` or `hello.wasm`
  loaded from a **file path** or an **HTTP URL** — no host rebuild to swap apps.
- **M5 — the shell is a browser.** A home screen lists the mini-apps; clicking one
  loads and runs it in the same window, and Home returns. Apps are swapped at
  runtime **inside a single running process** — no restart, no rebuild.

The `--script "frame:button,…"` flag injects clicks deterministically so the whole
host↔guest cycle is verifiable headlessly (no display required), e.g. the counter
going 0→1→2→3 then reset to 0.

## A note on the GUI: egui, not Makepad (yet)

Makepad was the intended primary toolkit. This MVP renders with **egui** instead,
for one technical reason: the mini-app boundary here is **immediate mode** (the
guest re-describes its entire UI every frame), and egui is an immediate-mode
toolkit, so it maps 1:1 with no adapter. Makepad is retained-mode + a DSL, which
would need a translation layer. Both are pure-Rust, GPU-rendered, and use **no
HTML/CSS/DOM/JS**, so the "kill the web stack" goal holds either way.

Because the renderer is isolated behind the `UiCmd` protocol (`ui.rs`) and the
`gui.rs` backend, switching to Makepad later is a localized change: implement the
same loop against `MiniApp::frame` — nothing else moves.

## Beyond the MVP (not built here)

- **Typed interfaces** via the WASM Component Model + WIT (instead of the hand-rolled
  `(ptr, len)` ABI).
- **Richer UI protocol**: layout, text input, images, a retained widget tree.
- **Per-mini-app capability policy** (which apps may call which host functions), fuel/
  memory limits, timeouts.
- **A mini-app SDK** so apps can be written in C / Go / etc., not just Rust.
- **Zero-copy bulk data** (shared linear-memory buffers for pixels/geometry) instead
  of copying strings across the boundary each call.

## License

MIT
