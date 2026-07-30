# nimadir — a native app shell that runs sandboxed WASM **component** mini-apps

An MVP of a "super-app / application-OS" idea: a **native** desktop shell that
loads **WebAssembly mini-apps on demand** and renders their UI natively, with
**zero web stack** — no HTML, no CSS, no JavaScript, no webview.

It behaves *like* a browser (a home screen of links, an address bar, fetch remote
code, run it sandboxed, expose capabilities) **without being one**. Third-party
mini-apps are WASM **components** fetched at runtime that talk to the host through
a small, typed, capability-scoped **WIT interface**. The host is a generic
renderer + capability provider; each mini-app owns its logic and state inside a
Wasmtime sandbox.

Everything is Rust: the host embeds **Wasmtime** (WASM runtime, Component Model)
and, for the window, **egui** (pure-Rust, GPU-rendered, no DOM).

```
┌───────────────────────────────────────────────────────────────┐
│  NATIVE RUST HOST (the shell)                                  │
│  window + GUI (egui) + Wasmtime + typed host interface         │
│                                                                │
│    ┌──────────────── Wasmtime component sandbox ───────────┐   │
│    │  mini-app.wasm  — a WASM *component* (untrusted)       │   │
│    │  exports:  init(), update()                           │   │
│    │  imports:  nimadir:shell/host-api                      │   │
│    │            (log, ui-label, ui-button, now-millis)      │   │
│    └────────────────────────────────────────────────────────┘  │
│           ▲   typed boundary — WIT + canonical ABI   ▲         │
│           │   (NO JavaScript, NO DOM, NO ptr/len)     │         │
└───────────────────────────────────────────────────────────────┘
                      NATIVE OS (Win / macOS / Linux)
```

## Why the Component Model (what changed)

The first cut used a hand-rolled `(ptr, len)` ABI over a raw core module: every
mini-app hand-declared `extern "C"` imports, passed strings as pointer+length,
and shipped a `guest_alloc` so the host could reach into its linear memory. That
worked, but it was **per-language and untyped** — nothing a browser's DOM isn't.

This version moves the boundary to the **WASM Component Model + WIT**:

- The contract lives **once**, in [`wit/world.wit`](wit/world.wit).
- The **guest** bindings are generated from it by `wit-bindgen`; the **host**
  bindings are generated from the *same file* by `wasmtime::component::bindgen!`.
- The Component Model's **canonical ABI** moves `string`/`bool`/… across the
  boundary. No `(ptr, len)`, no `guest_alloc`, no manual memory reads on the host.
- Because the interface is language-neutral, a mini-app can be written in **any
  language that targets components** (Rust today; C#/.NET via `componentize-dotnet`,
  Go via TinyGo, … next) with **no host change**.

That typed, shared, language-neutral interface is the "native DOM" the web gives
you through the browser — but here it is native, and free of JS/HTML/CSS.

## Quick start

```bash
# 1. one-time: the WASM target used to build mini-apps
rustup target add wasm32-unknown-unknown

# 2. build the mini-apps into COMPONENTS
#    (cargo build -> core module, then our `componentize` tool -> component)
./build-mini-apps.sh

# 3a. open the SHELL — a native window that starts on the home screen
cargo run -p host --features gui

# 3b. or open a listed app directly, skipping the home screen
cargo run -p host --features gui -- counter

# 3c. run HEADLESS (default; builds & runs anywhere, no system GUI libs)
cargo run -p host -- counter --script "1:0,2:0,3:0,5:1" --frames 8
```

The host also loads a mini-app straight from a URL — the "browser-like" part:

```bash
cargo run -p host -- https://example.com/some-mini-app.component.wasm
```

Point it at a different component (a listed name, a file, **or** a URL) and a
different mini-app runs — **with no host rebuild.**

## The home screen

Opened with no argument, the shell behaves like a browser start page: the
mini-apps listed in `apps.list` as links, plus an address bar for any path or
URL. Clicking a link fetches that component, instantiates it, and runs it in the
same window; **Home** drops it and goes back.

```
apps.list           # name | source (path or URL) | description
counter | mini-apps/counter/counter.component.wasm | A counter…
hello   | mini-apps/hello/hello.component.wasm     | A second app…
```

`cargo run -p host -- --list` prints the same list as text (there is no home
screen to click headlessly). A name from the list works anywhere a path does, so
`-- counter` and `-- mini-apps/counter/counter.component.wasm` are the same
request.

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

## The interface (host <-> mini-app boundary)

This is the entire contract, and it is one file — [`wit/world.wit`](wit/world.wit):

```wit
package nimadir:shell@0.1.0;

interface host-api {
    log:        func(msg: string);          // debug log to the host
    ui-label:   func(text: string);         // draw a label this frame
    ui-button:  func(text: string) -> bool; // draw a button; true if clicked
    now-millis: func() -> s64;              // example host-owned capability
}

world mini-app {
    import host-api;          // the mini-app's capability set (all it can reach)
    export init:   func();    // one-time setup
    export update: func();    // called once per frame (immediate-mode UI)
}
```

- **`host-api`** is the capability table: a mini-app can call *only* what is
  listed here; nothing else crosses the sandbox. The host implements it in
  `host/src/host.rs` (`impl Host for HostState`).
- **`init` / `update`** are what the host calls on the guest. The UI is
  **immediate mode**: every frame the host calls `update()`, the guest calls
  `ui-label`/`ui-button` to describe what to show, and the host feeds click
  results back through `ui-button`'s return value.

## How to write your own mini-app

A mini-app is a `cdylib` crate targeting `wasm32-unknown-unknown`. All the
bindings come from the shared WIT — you write only behavior:

```rust
wit_bindgen::generate!({ world: "mini-app", path: "../../wit" });
use nimadir::shell::host_api::{ui_label, ui_button};

struct App;
impl Guest for App {
    fn init() {}
    fn update() {
        ui_label("my mini-app");
        if ui_button("click me") { ui_label("clicked!"); }
    }
}
export!(App);
```

Build it into a component (or add its name to `APPS` in `build-mini-apps.sh`):

```bash
cargo build --release --target wasm32-unknown-unknown \
  --manifest-path mini-apps/<app>/Cargo.toml
cargo run --release -p componentize -- \
  mini-apps/<app>/target/wasm32-unknown-unknown/release/<app>.wasm \
  mini-apps/<app>/<app>.component.wasm
```

Then add a line to `apps.list` so it shows on the home screen. See
`mini-apps/counter` (guest-owned state) and `mini-apps/hello` (a second app) for
complete examples.

## Layout

```
wit/world.wit              # THE contract — one shared source of truth for both sides
Cargo.toml                 # workspace = [host, tools/componentize]
apps.list                  # the home screen's links ("bookmarks")
host/
  src/main.rs              # CLI, arg parsing, headless loop, --list
  src/host.rs              # Wasmtime component embedding + generated host-interface impl
  src/ui.rs                # UiCmd / FrameInput — the renderer-agnostic UI protocol
  src/registry.rs          # apps.list parsing; name -> source resolution
  src/shell.rs             # navigation (Home <-> App), fetch-from-file/URL
  src/gui.rs               # native egui window backend (feature "gui")
tools/componentize/        # core-module -> WASM component encoder (wraps `wit-component`)
mini-apps/
  counter/                 # sample: a counter; state lives inside the guest
  hello/                   # sample: a second app, to show hot-swap without rebuild
build-mini-apps.sh         # build + componentize every mini-app
```

Why a `componentize` tool? `wit-bindgen` emits a core module with the world's
type embedded as custom sections; a component is that module *encoded* into the
Component Model. `wasm-tools component new` does this from the CLI, but to keep
the repo self-contained (no external binary on PATH) `tools/componentize` does
the same in ~15 lines using the `wit-component` library.

## MVP status — milestones verified (now over the Component Model)

- **M1 — native shell.** Host builds and runs; egui window backend builds (feature `gui`).
- **M2 — typed boundary, both directions.** Host implements `nimadir:shell/host-api`
  (`log` / `ui-label` / `ui-button` / `now-millis`); the guest imports it and exports
  `init` / `update`. The canonical ABI moves strings/bools — **no JS, no DOM, no
  `(ptr, len)`, no manual memory reads.**
- **M3 — immediate-mode UI + a real mini-app.** The `counter` mini-app draws a label
  and Increment/Reset buttons; the count lives **inside the WASM guest**, the host
  just renders and routes clicks.
- **M4 — on-demand loading.** The same host binary runs `counter.component.wasm` or
  `hello.component.wasm` loaded from a **file path** or an **HTTP URL** — no host
  rebuild to swap apps.
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
`gui.rs` backend, switching to Makepad later is a localized change: drive the same
`Shell` loop and translate its `UiCmd` stream — nothing else moves.

## Beyond the MVP (not built here)

- **A retained "native DOM"**: replace immediate-mode with a UI *tree* in WIT
  (nodes, attributes, `append-child`, patch, events). Matches the DOM mental
  model and cuts boundary traffic (send diffs, not the whole UI each frame).
- **Polyglot mini-apps**: a C#/.NET mini-app compiled to a component with
  `componentize-dotnet`, a Go one with TinyGo — same `wit/`, no host change.
- **Per-mini-app capability policy** (which apps may call which host functions),
  plus fuel/memory limits and timeouts.
- **Zero-copy bulk data** (shared linear-memory buffers for pixels/geometry)
  instead of copying across the boundary each call.
- **A Makepad backend** behind the same `UiCmd` protocol.

## License

MIT
