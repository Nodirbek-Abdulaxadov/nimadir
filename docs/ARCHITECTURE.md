# nimadir — internal code map

An orientation document for anyone (human or agent) about to change this repo.
It answers three questions: **what runs today**, **which file owns what**, and
**where new work has to attach**. It is written against symbol names rather
than prose summaries — and rather than line numbers, which drift with every
change and quietly start lying. Everything named here is greppable.

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
| Registry listing | `-- --list` | names the source it actually read |
| All three feature sets compile | `cargo build -p host [--features gui\|webview]` | OK, no warnings |

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

**The store** (`store.component.wasm`, 48 KB), headless:

| Command | Result |
|---|---|
| `-- store --frames 1` | 4 apps listed, read through `list-apps`, search field drawn |
| `-- store --input "1:0=hell" --script "2:0" --frames 4` | filtered to 1 of 4, button indices renumbered, click opened `hello` via `open-app` |
| `--registry <url> -- store` | catalogue fetched over HTTP and listed |
| `--registry <unreachable>` | fell back to the built-in list, did not abort |

**The window**, run under Xvfb with Mesa's software Vulkan and openbox, driven
with `xdotool` and screenshotted at each step:

| Step | Result |
|---|---|
| Start with no argument | the store, listing the registry, address bar above it |
| Type in the store's search box | filtered live to the one match |
| Click its Open button | `hello` running, address bar showing the resolved path |
| Home | back to the store, search cleared |
| Open a page URL | HTML+CSS rendered by WebKitGTK in-window, chrome intact above it |
| Click inside the page | JavaScript ran — input reaches the webview, the GTK pump works |
| Home, from a page | webview destroyed, clean start page, no leftover surface |
| App → type a URL → Enter | round trip back into the webview, title and status following |
| A registry whose store will not load | built-in list, red error naming the reason, shell alive |
| A failed navigation | error in red in the chrome, shell stays where it was |

Two things worth carrying forward:

- **`counter-cs` is skipped** — `dotnet` is not on PATH. The build script skips
  it cleanly by design; the Rust apps still build. Nothing is broken.
- The CLI panics with exit 101 if its stdout pipe closes early (`| head -n`).
  Pre-existing, unrelated to routing; noted so it is not misread as a fault.

---

## 2. The one-paragraph model

The host is a **frame pump around a sandbox**. Each frame it hands the guest
what the user did to the last frame's widgets, calls `update()`, and collects
the `ui-*` calls the guest made into a `Vec<UiCmd>`. Something then draws that
vector — the egui window, or `println!` in headless mode. The guest never
touches a screen; the host never knows what an app *means*. That seam is the
whole design, and it is why the start page can be an app rather than host code.

```
  FrameInput { clicked, edits }        ← what the user did to the last frame
            │
            ▼
   Shell::frame ──► MiniApp::frame ──► world.call_update()  [wasmtime sandbox]
            │                               │
            │        guest calls ui-label / ui-button / ui-text-edit,
            │        and may ask for a navigation it does not get to perform
            ▼                               │
      Vec<UiCmd>  ◄─────────────────────────┘
            │           + take_pending_open()  → Shell navigates, after the call
            │
            ├──► gui.rs   → egui widgets   (feature "gui")
            └──► main.rs  → stdout text    (headless, the test harness)
```

Alongside it sits the *other* kind of destination: a web page, drawn by a native
child webview into the same page area (§4.2). It shares nothing with the pump
above — no `UiCmd`, no sandbox, no WIT. Two modules, one window.

---

## 3. File-by-file

### `wit/world.wit` — the contract
The only place the host↔guest boundary is defined. `wit-bindgen` generates the
guest side from it, `wasmtime::component::bindgen!` the host side.

- `interface host-api` — the **capability table**, and the entire list of things
  a mini-app can reach: `log`, `ui-label`, `ui-button`, `ui-text-edit`,
  `now-millis`, `list-apps`, `open-app`, plus the `app-entry` record.
- `world mini-app` — imports `host-api`, exports `init` and `update`.

The last two are what let the start page be an app instead of host code: a guest
can reach neither disk nor network, so `list-apps` hands it the registry, and it
cannot navigate synchronously, so `open-app` is a request (§4.3).

Changing this file changes both sides at once. That is the point, and also the
reason to change it carefully.

### `host/src/ui.rs` — the renderer-agnostic protocol
- `enum UiCmd` — `Label`, `Button { index, text }`, `TextEdit { index, text }`.
  This is the *entire* vocabulary a mini-app can express.
- `struct FrameInput` — `clicked: HashSet<u32>` and `edits: HashMap<u32, String>`.
  Both describe what the user did to the *previous* frame's widgets. That
  one-frame lag is inherent: the guest asks for a widget and gets its result in
  the same call, so the only result the host can have is the last one drawn.

Small file, high leverage: it is the seam that lets the same guest render on
egui, on stdout, or on a future backend. Any new widget type starts here.

### `host/src/host.rs` — the sandbox boundary
- `mod bindings` — `bindgen!` on `../wit`, kept in a submodule so the
  generated `MiniApp` world type doesn't collide with the wrapper below.
- `struct HostState` — per-instance host state: the `ui` command buffer,
  this frame's `input`, the `button_counter`, `logs`, plus a deliberately
  minimal `WasiCtx` (stderr only; no filesystem or network preopens).
- `impl Host for HostState` — **the capability implementations**. Adding a host
  capability means adding a method here and a line in the WIT. `ui_button` and
  `ui_text_edit` are where widget indices are handed out, in call order.
- `list_apps` answers with a snapshot of the registry handed in at load time —
  a guest can reach neither disk nor network, so this is the only way the store
  learns what exists.
- `open_app` records into `pending_open` rather than acting; see §4.3.
- `MiniApp::load` — compile, link (`host-api` + WASI 0.2), instantiate,
  call `init`.
- `MiniApp::frame` — clear the buffer, set the input, `call_update`, return a
  clone of the collected commands.
- `MiniApp::take_pending_open` — the navigation the guest asked for, taken once
  it is off the stack and dropping it is legal.

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

- `enum View` — `Home`, `App { title, src, app }`, or `WebPage { title, url }`.
  `Home` is the fallback list, not the usual start page: that is the store,
  which is an `App` like any other.
- `struct Shell` — owns `engine`, `apps` (the registry), `view`, and a
  `status` / `status_is_error` pair used as a status bar.
- `Shell::open` — the single navigation entry point: classify with
  `resolve`, then dispatch on the answer. Failure leaves the previous view
  intact and records the error in `status`; a bad link must not take the shell
  down. The view is only replaced once the destination is known good.
- `Shell::go_home` — the **store mini-app is the start page** when the registry
  lists one; the built-in list is a fallback, not the destination. A start page
  that can be swapped is one that can be broken, so a missing or failing store
  falls back and says why rather than leaving the shell with nowhere to go.
  Replaces `view`, which **drops the `MiniApp`, its
  Wasmtime `Store`, and the guest's linear memory**. Reopening an app starts it
  from scratch. That is the sandbox working, not lost state.
- Fetching itself now lives in `resolve.rs`, since classification and loading
  are the same request.

### `host/src/registry.rs` — the bookmarks
- `struct AppEntry` — `name`, `src`, `description`. Knows nothing about
  WASM; it is an address book.
- `parse` / `parse_json` — the pipe format and the JSON catalogue. JSON is what
  a registry *server* would serve, so one parser reads a local file and a remote
  one; `load_from` takes a URL as readily as a path. Every failure falls back
  rather than propagating.
- `builtin` — fallback entries so a fresh clone has a working home screen.
  Note these are still *sources*, not classified targets; `resolve` runs after.
- `normalize` — strips surrounding quotes (Windows "Copy as path" pastes).
- `resolve` — `input -> (source, title)`. A bare registry name wins;
  anything else passes through unchanged. **Both the CLI and the address bar go
  through this**, so `counter` means the same thing in either place.

### `host/src/gui.rs` — the egui backend (feature `gui`)
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
- **Page** — `page` translates the guest's `Vec<UiCmd>` into egui
  widgets and records clicks into `pending_clicks` for the *next* frame.

Also: `run` calls `eframe::run_native`; `impl eframe::App` —
note eframe 0.35 hands `fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut Frame)`
directly, with no `CentralPanel` to open; `sync_window_title` mirrors the
view into the OS title bar like a browser tab.

### `host/src/webview.rs` — module 1 (feature `webview`)
The only file in the repo where HTML, CSS and JavaScript exist, quarantined on
purpose.

- `WebPane` — a child webview built into the eframe window via
  `build_as_child`, occupying exactly the page area. `update` reconciles URL and
  bounds each frame, reloading or resizing only on an actual change, so a still
  frame costs nothing and a window resize needs no resize plumbing.
- `init` / `pump` — GTK's loop on Linux, no-ops elsewhere. Without the pump a
  page loads and then freezes. The pump is budgeted at 64 iterations so a busy
  page cannot starve the egui frame it is drawn inside.
- `check_parent` — rejects a Wayland surface with a readable error instead of
  letting wry panic on it.
- Dropping the pane destroys the native surface, which is what the shell does on
  navigation. See invariant 3.

### `host/src/main.rs` — CLI and headless harness
- `main` — hand-rolled arg parsing (`--list`, `--headless`, `--frames`,
  `--script`, plus one positional source). A positional argument means "navigate
  straight there", skipping the home screen.
- `parse_script` — `"1:0,2:0"` → `{frame: {button indices}}`.
- `run_headless` — runs N frames, injects the scripted clicks, prints
  every `UiCmd` and guest log line.

`run_headless` is the project's test suite. There is no `#[test]` anywhere; the
`--script` flag *is* how the boundary is verified, and it needs no display.

### `tools/componentize/` — core module → component
Wraps `wit_component::ComponentEncoder`. Does what `wasm-tools component new`
does, so the repo needs no external CLI on PATH.

### `mini-apps/store/` — the start page, as a guest
The catalogue: lists what `list-apps` returns, filters it against a `ui-text-edit`
field, and calls `open-app` on the one you pick. No privileges the counter lacks.
Its existence is the argument: the host no longer owns a home screen, it owns a
*fallback*, and the start page is now a component that can be replaced without
touching the shell.

### `mini-apps/` — the other guests
`counter` (Rust, state in an `AtomicI32`), `hello` (Rust, second app to prove
hot-swap), `counter-cs` (C#, same WIT, no host change). Each is a `cdylib`
excluded from the workspace — they target `wasm32-unknown-unknown` and import
host functions that only exist at runtime, so they must not be built for the
native host target.

---

## 4. Findings, and what became of them

Four things the code turned out to require that reading the plan alone would not
have told you. Each is kept with its outcome, because the reasoning is what
transfers — the next widget, the next capability, the next platform backend will
hit the same walls.

### 4.1 Content-type routing — built
`resolve.rs` and `View::WebPage` are this note, implemented. One thing it did
*not* solve, deliberately: `registry::resolve` still has no notion of a
scheme, so a bare `example.com` is treated as a path and reports that a web
address needs its `https://`. Guessing a scheme is address-bar smartness and
belongs with the rest of it, not smuggled in here.

### 4.2 Webview: eframe did not have to be replaced — confirmed
The build plan flagged "eframe hides its event loop" as the project's largest
risk, with a full `winit` + `wgpu` rewrite as the fallback. That rewrite was not
needed, and the reason is three lines of API:

- `eframe-0.35.0/src/epi.rs:101` — `impl HasWindowHandle for CreationContext<'_>`
- `eframe-0.35.0/src/epi.rs:695` — `impl HasWindowHandle for Frame`
- `wry-0.56.1/src/lib.rs:1571` — `pub fn build_as_child<W: HasWindowHandle>(self, window: &'a W) -> Result<WebView>`

`Frame` is handed to `App::ui` every frame, so the handle wry needs is already
in reach, and `load_url` / `set_bounds` do the rest. This turned out **better**
than the plan hoped for: the plan expected whole-window mode switching, with
egui hidden while a page was open. Because the child webview can be given
arbitrary bounds, only the *page area* switches — the address bar and Home
button stay drawn by egui and keep working over a live page.

The real cost is **Linux**, and wry documents it on `build_as_child` itself:

- X11 only — *"This method won't work on Wayland."*
- webkit2gtk still has to be initialised: call `gtk::init()` on the same thread
  and pump `gtk::main_iteration_do` alongside the event loop.
- it **panics** on Linux if `gtk::init` was not called on that thread.

So the plan's "both live in one winit event loop" is accurate on Windows and
macOS, where the webview is an OS-level child surface; on Linux there is a
second (GTK) loop that must be pumped by hand. Practical consequences:

1. The webview is its own **optional cargo feature**, exactly like `gui`.
   `libwebkit2gtk-4.1-dev` is a system package, and making it mandatory would
   break the default headless build.
2. Wayland is rejected up front. `webview::check_parent` inspects the raw handle
   and returns an error naming XWayland as the way out, because handing a
   Wayland surface to `build_as_child` is a documented *panic*, and a shell that
   dies because of the display server it happens to run under is not a shell.
3. The failure is remembered (`ShellApp::web_error`), so a webview that cannot
   exist is reported once instead of retried sixty times a second.

**A known environment failure, recorded so it is not re-diagnosed.** Under Xvfb
with Mesa's software stack (no DRI3), WebKitGTK's GL compositing emits
`GLXBadWindow`. Xlib reports errors asynchronously, so it surfaces at the next
sync point — which is `winit`'s `set_title`, and winit `.expect()`s there, so
the process aborts on the *next window-title change after a page is open*. It is
not nimadir's code: setting `WEBKIT_DISABLE_COMPOSITING_MODE=1` makes it go away
entirely, and it needs a GL stack broken enough to fail in the first place.
Worth knowing because the crash points at a title change and the cause is a
webview, several frames earlier.

### 4.3 A guest cannot navigate synchronously — built as predicted
`MiniApp::frame` calls `world.call_update(&mut self.store)`, which holds `&mut`
on the store for the whole guest call. An app asking the host to open another
app therefore **cannot** be honoured inside that call: dropping the running
`MiniApp` from inside its own `update()` is not expressible in Rust, and would
not be sane if it were.

So `open-app` records into `HostState::pending_open`, and `Shell::frame` drains
it after `call_update` returns — the same shape as `take_logs`. Two consequences
worth knowing when reading the code:

- `Shell::frame` returns an **empty** command list on the frame a navigation
  happened. The commands it collected describe the app that just asked to
  leave; drawing them would paint one frame of an app the shell has dropped.
- `Shell::navigated` is a read-and-clear flag so a backend can tell "the UI
  changed because the app changed" from "the UI changed because the app
  redrew", and throw away widget state belonging to the old instance.

### 4.4 The UI vocabulary needed a third widget — built
`UiCmd` was `Label` and `Button`, with no text input, so the store's search box
could not be written at all. `TextEdit` is the third, and it cost exactly what
was predicted: `wit/world.wit`, `ui.rs`, `host.rs`, `gui.rs`, and the headless
harness (`--input`, mirroring `--script`).

**The bug that fell out of it, because it will recur for any future widget:**
egui keys focus and cursor position by widget id, and `ui.text_edit_singleline`
derives that id from draw order. A guest's widget list changes shape constantly
— the store's search filters the rows *below* the search box — so the id moved
out from under the field between frames and focus dropped on the first
keystroke. The field rendered perfectly and simply could not be typed into.
`gui.rs` now pins the id to the guest's field index. Any stateful widget added
later needs the same treatment: **guest-assigned index, not host draw order.**

Icons were considered and dropped: `gui.rs` already documents that egui's
bundled fonts have no arrow or bullet glyphs and render missing ones as tofu
boxes, so an emoji column would have been a column of boxes.

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
