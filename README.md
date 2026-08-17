# nimadir — a native app shell that runs sandboxed WASM **component** mini-apps

An MVP of a "super-app / application-OS" idea: a **native** desktop shell that
loads **WebAssembly mini-apps on demand** and renders their UI natively, with
**zero web stack** in the app path — no HTML, no CSS, no JavaScript, no DOM.

It behaves *like* a browser (a start page, an address bar, fetch remote code, run
it sandboxed, expose capabilities) **without being one**. Third-party mini-apps
are WASM **components** fetched at runtime that talk to the host through a small,
typed, capability-scoped **WIT interface**. The host is a generic renderer +
capability provider; each mini-app owns its logic and state inside a Wasmtime
sandbox. Even the start page is one of these apps.

There is now a second, optional module for **the old web**: a page you point the
address bar at opens in the platform's own webview. That is a compatibility
shim, not a retreat — it is a separate cargo feature, confined to one file, and
mini-apps cannot reach it. The claim is unchanged where it counts: **writing an
app for nimadir involves no web stack at all.**

The host is Rust: it embeds **Wasmtime** (WASM runtime, Component Model) and, for
the window, **egui** (pure-Rust, GPU-rendered, no DOM). Mini-apps can be written
in **any language that compiles to a component** — this repo ships samples in
both **Rust** and **C# (.NET)**, run by the same host with no changes.

```
┌───────────────────────────────────────────────────────────────┐
│  NATIVE RUST HOST (the shell)                                  │
│  window + GUI (egui) + Wasmtime + typed host interface         │
│                                                                │
│    ┌──────────────── Wasmtime component sandbox ───────────┐   │
│    │  mini-app.wasm  — a WASM *component* (untrusted)       │   │
│    │  exports:  init(), update()                           │   │
│    │  imports:  nimadir:shell/host-api                      │   │
│    │            (log, ui-label, ui-button, ui-text-edit,    │   │
│    │             ui-heading, ui-search-field, ui-tile,      │   │
│    │             now-millis, list-apps, open-app)           │   │
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
  language that targets components** — this repo has both **Rust** and **C#/.NET**
  mini-apps (see *Polyglot*, below), Go via TinyGo next — all talking to the
  **same host, unchanged**.

That typed, shared, language-neutral interface is the "native DOM" the web gives
you through the browser — but here it is native, and free of JS/HTML/CSS.

## Quick start

```bash
# 1. one-time: the WASM target used to build mini-apps
rustup target add wasm32-unknown-unknown

# 2. build the mini-apps into COMPONENTS (Rust via cargo + componentize; the C#
#    app too, if the .NET 10 SDK is installed — otherwise it is skipped cleanly)
./build-mini-apps.sh

# 3a. open the SHELL — a native window that starts on the store
cargo run -p host --features gui

#     with the webview module too, so web pages open in-window
cargo run -p host --features webview

# 3b. or open a listed app directly, skipping the store
cargo run -p host --features gui -- counter

# 3c. run HEADLESS (default; builds & runs anywhere, no system GUI libs)
cargo run -p host -- counter --script "1:0,2:0,3:0,5:1" --frames 8

# 4. run the tests
./run-tests.sh
```

The host also loads a mini-app straight from a URL — the "browser-like" part:

```bash
cargo run -p host -- https://example.com/some-mini-app.component.wasm
```

Point it at a different component (a listed name, a file, **or** a URL) and a
different mini-app runs — **with no host rebuild.**

## The start page

Opened with no argument, the window comes up on the **store** — a mini-app that
lists the registry and filters it as you type. Picking an app fetches that
component, instantiates it, and runs it in the same window; **Home** drops it and
returns to the store.

```
apps.json           # the catalogue: name, source (path or URL), description
apps.list           # the same, pipe-delimited; used when apps.json is absent
```

`cargo run -p host -- --list` prints the registry as text (there is no start page
to click headlessly). A name from the registry works anywhere a path does, so
`-- counter` and `-- mini-apps/counter/counter.component.wasm` are the same
request.

### The address bar routes

The address bar is chrome — it sits above the page in every view *except the
start page*, so a new address can be typed without going home first. What you
type is not assumed to be a mini-app: `resolve.rs` classifies it, and the shell
dispatches on the answer.

The start page is the exception because it already has a text field of its own,
and two stacked bars that mean different things is worse than either. There, the
store's search box takes an address too and offers it as an "Open address" card;
clicking one calls `open-app`, so the text still reaches the same resolver.
Everywhere else the bar is back, showing the URL or the app's name.

```
a registry name   -> that entry's source            ("counter")
already a URL     -> untouched                      ("https://…/app.wasm")
an existing file  -> untouched                      ("mini-apps/…/app.wasm")
host-shaped       -> https:// in front of it        ("example.com")
```

then, on whatever that produced:

```
local path        -> by extension (.wasm / .html), no network at all
URL ending .wasm  -> a component; the address already said so
any other URL     -> fetched once, classified by Content-Type
```

Guessing a scheme is the *last* of those steps on purpose: a registry name and a
real file both beat it, so a guess can never shadow something that exists. It
looks only at the authority — the part before the first `/` — so `app.wasm` stays
a filename that happens to contain a dot, rather than becoming a DNS failure.

`text/html` routes to a web page; `application/wasm` routes to the sandbox. The
classification fetch *is* the load — a component is never downloaded twice, which
is why there is no separate `HEAD` probe. Servers that mislabel `.wasm` as
`application/octet-stream` are still handled: the bytes are already in hand, and
the WASM magic number settles it. Anything that is neither is an error you can
read, on the screen you were already on.

### Two modules, one window

A web page is rendered by the platform's own webview (`wry` — **not** Tauri,
which would bring an application framework that fights the host loop nimadir
already owns). It is built as a **child** of the eframe window covering exactly
the page area, so the chrome above stays egui's and keeps working while a page
is open.

```bash
cargo run -p host --features webview -- https://example.com
```

The webview is a separate feature from `gui` because it needs system libraries
`gui` does not — WebKitGTK on Linux (`libwebkit2gtk-4.1-dev`), WebView2 on
Windows. The default build stays buildable anywhere, which is what keeps the
headless path honest.

The rules that fall out of this are worth stating, because they are the reason
the "no web stack" goal survives having a webview at all:

- **Module 1 is quarantined.** HTML, CSS and JavaScript exist in `webview.rs`
  and nowhere else. Mini-apps are unaffected: they are WASM components that
  describe native widgets, and nothing about them changes because this file
  exists. A page is a *different kind of destination*, not a new way to write
  an app.
- **One module at a time in the page area.** The webview is a native surface
  the OS stacks over that region; egui cannot draw into it, and it cannot draw
  outside it. That is why the chrome lives above the page area rather than in it.
- **Leaving a page destroys it.** The webview is dropped on navigation, exactly
  as a mini-app's `Store` is. A hidden-but-alive webview would keep running
  scripts, timers and audio behind a screen that says you left.

On Linux `wry` is WebKitGTK, which lives in **GTK's** event loop rather than
winit's, so the host initialises GTK once and pumps it each frame; without that
a page loads and then freezes. Child webviews there are also X11-only — a
Wayland session is rejected with a message saying so, rather than being handed
to wry, which would panic on it.

Two properties fall out of the design and are worth stating:

- **Chrome and page are separate.** The address bar and Home button are host UI;
  a guest only ever paints into the page area below them. Navigation is never
  reachable by untrusted code — the store asks for it through `open-app` and the
  host decides, rather than performing it itself.
- **Leaving an app destroys it.** Navigating home drops the `MiniApp`, its
  Wasmtime `Store`, and the guest's linear memory. Reopening `counter` starts at
  0 again — the sandbox doing its job, not lost state.

An address that fails to load (missing file, dead URL, invalid module, a page
that is neither) leaves the shell where it was with the error shown in red. A bad
link cannot take the shell down.

## The interface (host <-> mini-app boundary)

This is the entire contract, and it is one file — [`wit/world.wit`](wit/world.wit):

```wit
package nimadir:shell@0.1.0;

interface host-api {
    record app-entry { name: string, source: string, description: string }

    log:             func(msg: string);            // debug log to the host
    ui-label:        func(text: string);           // draw a label this frame
    ui-button:       func(text: string) -> bool;   // draw a button; true if clicked
    ui-text-edit:    func(text: string) -> string; // draw a field; edited value back
    ui-heading:      func(text: string, level: u8);            // 1 is the largest
    ui-search-field: func(text: string, placeholder: string) -> string;
    ui-tile:         func(title: string, subtitle: string) -> bool; // a card in a grid
    now-millis:      func() -> s64;                // example host-owned capability
    list-apps:       func() -> list<app-entry>;    // the registry, for the store
    open-app:        func(source: string);         // ask the shell to navigate
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
  `ui-label`/`ui-button`/`ui-text-edit` to describe what to show, and the host
  feeds results back through those calls' return values. State lives in the
  guest; the host owns only the live widget buffers for one frame.
- **Every widget names a *thing*, never a position.** `ui-heading` says "this is
  a heading", `ui-tile` says "this is a card"; how large a heading is and how
  many cards fit on a row are the host's business. That is what keeps the
  renderer swappable — a guest that could place pixels would be a guest egui
  could never be swapped out from under. It is also what lets the start page
  look like a browser's without the sandbox learning a single thing about egui.
- **`list-apps` / `open-app`** are what make the start page an app rather than
  host code. A guest can reach neither disk nor network, so the registry is
  handed to it; and it cannot navigate synchronously, because the host cannot
  tear a guest down in the middle of its own `update`. `open-app` is therefore a
  *request*, honoured once the frame returns.

## The store is a mini-app

The window opens on `mini-apps/store` — a catalogue that lists the registry,
filters it as you type, and asks the shell to open whatever you pick. **Home**
returns to it.

It is an ordinary guest. Same sandbox, same `wit/world.wit`, no privileges the
counter lacks; it is 48 KB of WASM the host fetches like any other. The host
stopped owning the start page and now just runs whichever app is pointed at it —
one of which happens to be the catalogue. Replacing the start page means
shipping a different component, not patching the shell.

If the store is missing or fails to load, the shell falls back to its built-in
link list and says why. A start page that can be swapped is also a start page
that can be broken, so it cannot be the only way home.

```bash
cargo run -p host -- store --frames 1                       # what it lists
cargo run -p host -- store --input "1:0=hell" --script "2:0" --frames 4
#   frame 1 types "hell" into field 0, frame 2 clicks the one result: hello opens
```

### Where the registry comes from

`apps.json` if present, else `apps.list`, else built-in entries. JSON is what a
registry *server* would serve, so the same parser reads a local catalogue and a
remote one:

```bash
cargo run -p host -- --registry https://example.com/catalogue.json --list
```

A registry that will not load falls back rather than aborting — a shell that
refuses to open because its bookmarks are malformed is worse than one that opens
with the built-in list.

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

Then add an entry to `apps.json` so the store lists it. See
`mini-apps/counter` (guest-owned state) and `mini-apps/hello` (a second app) for
complete examples.

## Polyglot: the same counter in C# and Python

The payoff of a typed, language-neutral WIT interface is that the host doesn't
care what language a mini-app is written in. `mini-apps/counter-cs` is the
`counter`, rewritten in **C#** and compiled to a WASM component with
[`componentize-dotnet`](https://github.com/bytecodealliance/componentize-dotnet)
(NativeAOT-LLVM). The **host is not changed** to run it — the C# side generates
its bindings from the very same `wit/world.wit`:

```csharp
using MiniAppWorld;
using static MiniAppWorld.wit.Imports.nimadir.shell.v0_1_0.IHostApiImports;

public class MiniAppWorldExportsImpl : IMiniAppWorldExports
{
    private static int _count;
    public static void Init()   => Log($"counter-cs: init at host-time {NowMillis()}");
    public static void Update()
    {
        UiLabel($"Count (C#): {_count}");
        if (UiButton("Increment")) _count++;
        if (UiButton("Reset"))     _count = 0;
    }
}
```

Two things are worth knowing:

- **The host provides WASI.** A language with a runtime (C#, Go, …) imports the
  `wasi:*` interfaces (clocks, random, `io/poll`, …) even for a headless reactor,
  so the host adds WASI 0.2 to the component linker (`wasmtime-wasi`). The Rust
  mini-apps import none of it, so this is invisible to them — it's purely
  additive. The WASI context is minimal: no filesystem or network preopens.
- **Size is the trade-off.** See the table below: the runtime travels inside the
  component, and that is the reason to **mix** — tiny Rust widgets next to
  heavier managed apps, all in one shell.

Build it with the **.NET 10 SDK** (`build-mini-apps.sh` does this automatically
when `dotnet` is on PATH; the NativeAOT-LLVM + WASI SDK toolchain is downloaded
and cached on the first build):

```bash
dotnet build -c Release mini-apps/counter-cs/counter-cs.csproj
# -> mini-apps/counter-cs/bin/Release/net10.0/wasi-wasm/publish/counter_cs.wasm
cargo run -p host -- counter-cs --script "1:0,2:0,3:0,5:1" --frames 8
```

### …and in Python

`mini-apps/counter-py` is the third one, built with
[`componentize-py`](https://github.com/bytecodealliance/componentize-py), which
bakes a **whole CPython interpreter** into the component. Again: the host is not
changed, and the bindings come from the same `wit/world.wit`.

```python
import wit_world
from wit_world.imports.host_api import log, now_millis, ui_button, ui_heading, ui_label

class WitWorld(wit_world.WitWorld):
    count = 0

    def init(self) -> None:
        log(f"counter-py: init at host-time {now_millis()}")

    def update(self) -> None:
        ui_heading("Count (Python)", 2)
        ui_label(str(WitWorld.count))
        if ui_button("Increment"):
            WitWorld.count += 1
        if ui_button("Reset"):
            WitWorld.count = 0
        # `datetime` is the real stdlib, running inside the sandbox.
        stamp = datetime.fromtimestamp(now_millis() / 1000, timezone.utc)
        ui_label(f"host clock via Python datetime: {stamp:%H:%M:%S} UTC")
```

That last line is the part worth pausing on: `now-millis` is the only thing the
host provides, and `datetime` formatting it is CPython's own standard library
executing in the Wasmtime sandbox.

| Mini-app | Language | Component |
|---|---|---|
| `counter` | Rust | ~19 KB |
| `counter-cs` | C# (NativeAOT-LLVM) | ~2.2 MB |
| `counter-py` | Python (CPython) | ~18 MB |

Three orders of magnitude, one unchanged host, one WIT file.

```bash
pip install componentize-py
componentize-py -d wit -w mini-app componentize app \
  -p mini-apps/counter-py -o mini-apps/counter-py/counter-py.component.wasm
cargo run -p host -- counter-py --script "1:0,2:0,3:0,5:1" --frames 8
```

## Layout

```
wit/world.wit              # THE contract — one shared source of truth for both sides
Cargo.toml                 # workspace = [host, tools/componentize]
apps.json / apps.list      # the app registry ("bookmarks")
host/
  src/main.rs              # CLI, arg parsing, headless loop, --list
  src/host.rs              # Wasmtime component embedding, host-interface impl, WASI (wasmtime-wasi)
  src/ui.rs                # UiCmd / FrameInput — the renderer-agnostic UI protocol
  src/registry.rs          # apps.list parsing; name -> source resolution
  src/resolve.rs           # address classification (component vs web page) + fetch
  src/shell.rs             # navigation (Home <-> App <-> WebPage)
  src/gui.rs               # native egui window backend (feature "gui")
  src/webview.rs           # module 1: the old web via wry (feature "webview")
tools/componentize/        # core-module -> WASM component encoder (wraps `wit-component`)
mini-apps/
  store/                   # the start page: the app catalogue, itself a mini-app
  counter/                 # sample (Rust): a counter; state lives inside the guest
  hello/                   # sample (Rust): a second app, to show hot-swap without rebuild
  counter-cs/              # sample (C#):   the counter via componentize-dotnet — same WIT
build-mini-apps.sh         # build + componentize every mini-app (Rust + optional C#)
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
- **M5 — the shell is a browser.** A start page lists the mini-apps; picking one
  loads and runs it in the same window, and Home returns. Apps are swapped at
  runtime **inside a single running process** — no restart, no rebuild.
- **M6 — polyglot.** The same host runs a mini-app written in **C# (.NET)**
  (`counter-cs`), compiled to a component with `componentize-dotnet` and loaded
  with **no host change** — proof that the WIT interface, not the language, is the
  contract. (The host provides WASI 0.2 for the managed runtime; see *Polyglot*.)

- **M7 — the address bar routes.** What you type is classified before the shell
  navigates: a component goes to the sandbox, a web page to the webview, and
  anything else is a readable error rather than a failed load.
- **M8 — two modules, one window.** Web pages render in a native child webview
  under the same chrome, quarantined to one file and one cargo feature.
- **M9 — the start page is an app.** The store lists and searches the registry
  from inside the sandbox and asks the host to navigate. The host no longer owns
  a home screen; it owns a fallback.

## Tests

```bash
./run-tests.sh          # everything
./run-tests.sh -v       # with each command's full output
```

There is no `cargo test` here, deliberately. What needs proving is the
host↔guest boundary and the shell's routing, and both are exercised end to end
through the real binary — no display required, because `--script
"frame:button,…"` and `--input "frame:field=text"` inject clicks and typing
deterministically. The suite covers the counter going 0→1→2→3 then reset, the
store filtering to one result and opening it through `open-app`, every branch of
address classification, the scheme guess, a served registry, and all three
feature sets building.

Network cases use a throwaway `python3 -m http.server`; without python3, or if
its port is taken, they are **skipped** rather than failed — a suite that reports
failures for something it never ran sends people hunting for bugs that are not
there.

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
- **More languages**: a Go mini-app with TinyGo, and others — same `wit/`, no host
  change. (A **C#/.NET** mini-app is already built — see *Polyglot*, above.)
- **Per-mini-app capability policy** (which apps may call which host functions),
  plus fuel/memory limits and timeouts.
- **Zero-copy bulk data** (shared linear-memory buffers for pixels/geometry)
  instead of copying across the boundary each call.
- **A Makepad backend** behind the same `UiCmd` protocol.

## License

MIT
