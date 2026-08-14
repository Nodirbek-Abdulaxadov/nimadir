# nimadir — internal code map

An orientation document for anyone (human or agent) about to change this repo.
It answers three questions: **what runs today**, **which file owns what**, and
**where new work has to attach**. It is deliberately written against line
numbers and type names, not prose summaries.

Companion to `README.md`, which explains *why* the project exists. This file
explains *how the code is laid out*.

---

## 1. Verified behaviour

What has actually been run against this tree, rather than what it ought to do.

**The core, unchanged since the MVP:**

| Check | Command | Result |
|---|---|---|
| Mini-apps build | `./build-mini-apps.sh` | OK — `counter` 17 643 B, `hello` 11 380 B |
| Host + guest cycle | `-- counter --script "1:0,2:0,3:0,5:1" --frames 8` | count 0→1→2→3, reset to 0 |
| Second app, no host rebuild | `-- hello --frames 2` | OK |
| Registry listing | `-- --list` | 3 entries |
| Both feature sets compile | `cargo build -p host [--features gui]` | OK, no warnings |

**Address classification** (`resolve.rs`), against a local HTTP server:

| Address | Served as | Routes to |
|---|---|---|
| `…/counter.component.wasm` | `application/wasm` | component — 17 643 B, guest ran |
| `…/mystery` (no extension) | `application/octet-stream` | component — magic number settled it |
| `…/page.html` | `text/html` | web page |
| `…/notes.txt` | `text/plain` | error naming the type, exit 1 |
| `mini-apps/hello/hello.component.wasm` | local path | component |
| `…/page.html` (on disk) | local path | web page |
| `README.md` | local path | error, no network request |
| `example.com` | — | error asking for the scheme |

**The window**, run under Xvfb with Mesa's software Vulkan, screenshotted in
each of its three states: home (bookmark list), app (`counter` running under the
chrome), web page (placeholder), plus a failed navigation showing its error in
red while staying on home.

Three things worth carrying forward:

- **`counter-cs` is skipped** — `dotnet` is not on PATH. The build script skips
  it cleanly by design; the Rust apps still build. Nothing is broken.
- **No GTK.** `gtk+-3.0` and `webkit2gtk-4.1` are not installed. The egui window
  needs neither (winit talks to X11 directly), but the webview will — see §4.2.
- The CLI panics with exit 101 if its stdout pipe closes early (`| head -n`).
  Pre-existing, unrelated to routing; noted so it is not misread as a fault.

---

## 2. The one-paragraph model

The host is a **frame pump around a sandbox**. Each frame it hands the guest a
set of clicked button indices, calls the guest's `update()`, and collects the
`ui-label` / `ui-button` calls the guest made into a `Vec<UiCmd>`. Something
then draws that vector — the egui window, or `println!` in headless mode. The
guest never touches a screen; the host never knows what an app *means*. That
seam is the whole design.

```
  input: HashSet<u32> (clicked indices)
            │
            ▼
   Shell::frame ──► MiniApp::frame ──► world.call_update()  [wasmtime sandbox]
            │                               │
            │        guest calls ui-label / ui-button, host records them
            ▼                               │
      Vec<UiCmd>  ◄─────────────────────────┘
            │
            ├──► gui.rs   → egui widgets   (feature "gui")
            └──► main.rs  → stdout text    (headless, the test harness)
```

---

## 3. File-by-file

### `wit/world.wit` — the contract
The only place the host↔guest boundary is defined. `wit-bindgen` generates the
guest side from it, `wasmtime::component::bindgen!` the host side.

- `interface host-api` (:8) — the **capability table**. Four functions:
  `log`, `ui-label`, `ui-button -> bool`, `now-millis -> s64`. A mini-app can
  reach nothing else.
- `world mini-app` (:26) — imports `host-api`, exports `init` and `update`.

Changing this file changes both sides at once. That is the point, and also the
reason to change it carefully.

### `host/src/ui.rs` — the renderer-agnostic protocol (25 lines)
- `enum UiCmd` (:13) — `Label(String)` and `Button { index: u32, text: String }`.
  This is the *entire* vocabulary a mini-app can express today.
- `struct FrameInput` (:23) — `clicked: HashSet<u32>`.

Small file, high leverage: it is the seam that lets the same guest render on
egui, on stdout, or on a future backend. Any new widget type starts here.

### `host/src/host.rs` — the sandbox boundary (158 lines)
- `mod bindings` (:25) — `bindgen!` on `../wit`, kept in a submodule so the
  generated `MiniApp` world type doesn't collide with the wrapper below.
- `struct HostState` (:37) — per-instance host state: the `ui` command buffer,
  this frame's `input`, the `button_counter`, `logs`, plus a deliberately
  minimal `WasiCtx` (stderr only; no filesystem or network preopens).
- `impl Host for HostState` (:81) — **the capability implementations**. Adding a
  host capability means adding a method here and a line in the WIT.
  `ui_button` (:90) is where button indices are handed out, in call order.
- `MiniApp::load` (:116) — compile, link (`host-api` + WASI 0.2), instantiate,
  call `init`.
- `MiniApp::frame` (:143) — clear the buffer, set the clicks, `call_update`,
  return a clone of the collected commands.

WASI is linked in for runtime-bearing guest languages (C#, Go); the Rust
mini-apps import none of it, so it is purely additive.

### `host/src/resolve.rs` — the router
Decides what an address *is*, before the shell tries to go there.

- `enum Target` — `WasmApp(Vec<u8>)` (bytes included) or `WebPage`.
- `classify` — local paths by extension, `.wasm` URLs by name, everything else
  by a single `GET` whose `Content-Type` answers the question.
- The fetch is deliberate: the request a `HEAD` probe would save is the one the
  component path has to make anyway, so the body rides along with the answer and
  nothing is downloaded twice.
- `application/octet-stream` and missing types fall back to the WASM magic
  number, because plenty of static hosts mislabel `.wasm`.

### `host/src/shell.rs` — navigation
The browser-shaped layer above a single mini-app. Knows nothing about drawing.

- `enum View` (:28) — `Home`, `App { title, src, app }`, or
  `WebPage { title, url }`. The third is where the webview will render; today
  the backend draws a placeholder in it.
- `struct Shell` (:39) — owns `engine`, `apps` (the registry), `view`, and a
  `status` / `status_is_error` pair used as a status bar.
- `Shell::open` (:70) — the single navigation entry point: classify with
  `resolve`, then dispatch on the answer. Failure leaves the previous view
  intact and records the error in `status`; a bad link must not take the shell
  down. The view is only replaced once the destination is known good.
- `Shell::go_home` (:96) — replaces `view`, which **drops the `MiniApp`, its
  Wasmtime `Store`, and the guest's linear memory**. Reopening an app starts it
  from scratch. That is the sandbox working, not lost state.
- Fetching itself now lives in `resolve.rs`, since classification and loading
  are the same request.

### `host/src/registry.rs` — the bookmarks (111 lines)
- `struct AppEntry` (:12) — `name`, `src`, `description`. Knows nothing about
  WASM; it is an address book.
- `parse` (:40) — `name | src | description`, `#` comments and blank lines
  skipped.
- `builtin` (:65) — fallback entries so a fresh clone has a working home screen.
  Note these are still *sources*, not classified targets; `resolve` runs after.
- `normalize` (:90) — strips surrounding quotes (Windows "Copy as path" pastes).
- `resolve` (:105) — `input -> (source, title)`. A bare registry name wins;
  anything else passes through unchanged. **Both the CLI and the address bar go
  through this**, so `counter` means the same thing in either place.

### `host/src/gui.rs` — the egui backend (251 lines, feature `gui`)
Splits the window into two zones, and keeping them apart is the security story:

- **Chrome** — `chrome` draws the Home button, the **permanent address bar**,
  the title and the status line; `home` draws the registry links. Host UI. The
  guest cannot draw it, reach it, or know it exists. The address bar lives in
  the chrome rather than on the home screen because it is the entry point for
  both modules — you must be able to retype an address from inside an app.
- `navigate` is the one place a `Nav` becomes a `Shell::open` call, shared by
  the address bar and the home-screen links, so both accept the same input.
- `Body::of` picks the renderer for the area below the chrome. It reads the
  discriminant into a local first: the arms take `&mut self`, so the match
  cannot hold a borrow of `shell.view` across them.
- **Page** — `page` (:222) translates the guest's `Vec<UiCmd>` into egui
  widgets and records clicks into `pending_clicks` for the *next* frame.

Also: `run` (:38) calls `eframe::run_native`; `impl eframe::App` (:70) —
note eframe 0.35 hands `fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame)`
directly, with no `CentralPanel` to open; `sync_window_title` (:90) mirrors the
view into the OS title bar like a browser tab.

### `host/src/main.rs` — CLI and headless harness (172 lines)
- `main` (:36) — hand-rolled arg parsing (`--list`, `--headless`, `--frames`,
  `--script`, plus one positional source). A positional argument means "navigate
  straight there", skipping the home screen (:80).
- `parse_script` (:130) — `"1:0,2:0"` → `{frame: {button indices}}`.
- `run_headless` (:144) — runs N frames, injects the scripted clicks, prints
  every `UiCmd` and guest log line.

`run_headless` is the project's test suite. There is no `#[test]` anywhere; the
`--script` flag *is* how the boundary is verified, and it needs no display.

### `tools/componentize/` — core module → component (39 lines)
Wraps `wit_component::ComponentEncoder`. Does what `wasm-tools component new`
does, so the repo needs no external CLI on PATH.

### `mini-apps/` — the guests
`counter` (Rust, state in an `AtomicI32`), `hello` (Rust, second app to prove
hot-swap), `counter-cs` (C#, same WIT, no host change). Each is a `cdylib`
excluded from the workspace — they target `wasm32-unknown-unknown` and import
host functions that only exist at runtime, so they must not be built for the
native host target.

---

## 4. Where new work attaches

Notes gathered while reading the tree against the SuperBrowser build plan
(address bar routing → webview module → WASM app store). Recorded here because
they are properties of *this code*, not of the plan.

### 4.1 Content-type routing — built
`resolve.rs` and `View::WebPage` are this note, implemented. One thing it did
*not* solve, deliberately: `registry::resolve` (:105) still has no notion of a
scheme, so a bare `example.com` is treated as a path and reports that a web
address needs its `https://`. Guessing a scheme is address-bar smartness and
belongs with the rest of it, not smuggled in here.

### 4.2 Webview: eframe does not have to be replaced
The build plan flags "eframe hides its event loop" as the project's largest
risk, with a full `winit` + `wgpu` rewrite as the fallback. Reading both crates
says the fallback is probably unnecessary:

- `eframe-0.35.0/src/epi.rs:101` — `impl HasWindowHandle for CreationContext<'_>`
- `eframe-0.35.0/src/epi.rs:695` — `impl HasWindowHandle for Frame`
- `wry-0.56.1/src/lib.rs:1571` — `pub fn build_as_child<W: HasWindowHandle>(self, window: &'a W) -> Result<WebView>`

`Frame` is handed to `App::ui` every frame, so the handle wry needs is already
in reach. `WebView` also exposes `load_url` (`lib.rs:2213`) plus `set_bounds`,
`set_visible` and `focus` (`lib.rs:2263`–`2273`) — enough to keep the address bar drawn in egui at
the top and park the webview in the region below it, toggling visibility on mode
switch, rather than swapping whole windows.

The real cost is **Linux**, and wry documents it on `build_as_child` itself:

- X11 only — *"This method won't work on Wayland."*
- webkit2gtk still has to be initialised: call `gtk::init()` on the same thread
  and pump `gtk::main_iteration_do` alongside the event loop.
- it **panics** on Linux if `gtk::init` was not called on that thread.

So the plan's "both live in one winit event loop" is accurate on Windows and
macOS, where the webview is an OS-level child surface; on Linux there is a
second (GTK) loop that must be pumped by hand. Practical consequences:

1. The webview needs its own **optional cargo feature**, exactly like `gui`.
   `libwebkit2gtk-4.1-dev` is a system package (not installed here; apt
   candidate 2.50.4), and making it mandatory would break the default headless
   build that CI depends on.
2. A Wayland session needs a decision — force winit's X11 backend, or fall back
   to a separate top-level window via `build()` instead of `build_as_child()`.
3. None of it can be *runtime*-verified in this container: no display, no GTK.
   Compile-checking is possible after installing the system package.

### 4.3 A guest cannot navigate synchronously
`MiniApp::frame` (`host.rs:143`) calls `world.call_update(&mut self.store)`,
which holds `&mut` on the store for the whole guest call. A store app that asks
the host to open another app therefore **cannot** be honoured inside that call —
dropping the running `MiniApp` from inside its own `update()` is not expressible.

The shape that works: the capability records a request into `HostState`
(`pending_open: Option<String>`), and `Shell` drains it *after* `call_update`
returns and performs the navigation then. Same pattern as `take_logs` (:155).

### 4.4 The UI vocabulary is two widgets wide
`UiCmd` (`ui.rs:13`) is `Label` and `Button`. There is **no text input**. A store
app with a search field cannot be written against today's WIT — a search box
needs a new primitive (a text-edit command, plus a way to return the edited
string to the guest), which touches `wit/world.wit`, `ui.rs`, `host.rs`,
`gui.rs`, and the `--script` format used by the headless harness. Icons or
per-app colours would be further additions.

This is worth knowing early: it is a boundary change, and boundary changes are
the expensive kind here.

---

## 5. Invariants worth not breaking

1. **The WIT is the contract.** Add to it; do not reshape what exists. Both
   sides regenerate from the same file.
2. **Chrome is host-only.** Navigation must never be reachable by guest code.
3. **Leaving an app destroys it.** Do not add state that outlives the `Store`.
4. **Headless is the default feature set.** The host must keep building and
   running with no system GUI libraries; that is what makes it verifiable in CI.
   Every new system dependency goes behind a feature flag.
5. **Renderer stays behind `UiCmd`.** A backend swap should touch `gui.rs` and
   nothing else.
6. **No web stack in the mini-app path.** HTML/CSS/JS has no place on the guest
   side of the boundary.
