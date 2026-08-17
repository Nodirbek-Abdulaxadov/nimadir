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
| Mini-apps build | `./build-mini-apps.sh` | OK — `counter` 18 986 B, `hello` 15 551 B (rustc 1.97.1; the figure moves with the toolchain, which is why the suite reads it off the file) |
| Host + guest cycle | `-- counter --script "1:0,2:0,3:0,5:1" --frames 8` | count 0→1→2→3, reset to 0 |
| Second app, no host rebuild | `-- hello --frames 2` | OK |
| Registry listing | `-- --list` | names the source it actually read |
| All three feature sets compile | `cargo build -p host [--features gui\|webview]` | OK, no warnings |

**Address classification** (`resolve.rs`), against a local HTTP server:

| Address | Served as | Routes to |
|---|---|---|
| `…/counter.component.wasm` | `application/wasm` | component — 18 986 B, guest ran |
| `…/mystery` (no extension) | `application/octet-stream` | component — magic number settled it |
| `…/page.html` | `text/html` | web page |
| `…/notes.txt` | `text/plain` | error naming the type, exit 1 |
| `mini-apps/hello/hello.component.wasm` | local path | component |
| `…/page.html` (on disk) | local path | web page |
| `README.md` | local path | error, no network request |
| `example.com` | — | error asking for the scheme |

All of the headless cases below are `./run-tests.sh` — 36 assertions, exit
non-zero on failure, repeatable, cleaning up after itself. It is the suite; the
tables are what it covers.

**The store** (`store.component.wasm`, 43 KB), headless:

| Command | Result |
|---|---|
| `-- store --frames 1` | hero heading, search field, 3 app tiles read through `list-apps` — 4 registry entries, the store itself not among them |
| `-- store --input "1:0=hell" --script "2:0" --frames 4` | filtered to 1 of 3, tile indices renumbered, click opened `hello` via `open-app` |
| `-- store --input "1:0=example.com" --script "2:0" --frames 3` | offered an `Open address` tile; clicking it navigated through `open-app` |
| `--registry <url> -- store` | catalogue fetched over HTTP and listed |
| `--registry <unreachable>` | fell back to the built-in list, did not abort |

**The window**, run under Xvfb with Mesa's software Vulkan and openbox, driven
with `xdotool` and screenshotted at each step:

| Step | Result |
|---|---|
| Start with no argument | the store: wordmark, search box, grid of app cards — and no chrome above them (§4.7) |
| Type in the store's search box | filtered live to the one match |
| Click its card | `hello` running, address bar showing the resolved path |
| Home | back to the store, search cleared |
| Open a page URL | HTML+CSS rendered by WebKitGTK in-window, chrome intact above it |
| Click inside the page | JavaScript ran — input reaches the webview, the GTK pump works |
| Home, from a page | webview destroyed, clean start page, no leftover surface |
| App → type a URL → Enter | round trip back into the webview, title and status following |
| A registry whose store will not load | built-in list, red error naming the reason, shell alive |
| A failed navigation | error in red in the chrome, shell stays where it was |
| store → app → Home → web page → Home | every transition clean, no leftover surface, shell alive |

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
  `ui-heading`, `ui-search-field`, `ui-tile`, `now-millis`, `list-apps`,
  `open-app`, plus the `app-entry` record.
- `world mini-app` — imports `host-api`, exports `init` and `update`.

The last two are what let the start page be an app instead of host code: a guest
can reach neither disk nor network, so `list-apps` hands it the registry, and it
cannot navigate synchronously, so `open-app` is a request (§4.3).

Changing this file changes both sides at once. That is the point, and also the
reason to change it carefully.

### `host/src/ui.rs` — the renderer-agnostic protocol
- `enum UiCmd` — `Label`, `Button { index, text }`, `TextEdit { index, text }`,
  `Heading { text, level }`, `Search { index, text, placeholder }`,
  `Tile { index, title, subtitle }`. This is the *entire* vocabulary a mini-app
  can express, and every variant names a **thing**, never a position: `Tile`
  carries no coordinates, and a run of them becomes a grid only once `gui.rs`
  decides how many fit. `Search` shares the text-field index space with
  `TextEdit`, and `Tile` shares the button index space with `Button`, so a guest
  that swaps one for the other keeps its indices — and so do the `--script` /
  `--input` runs that drive it headlessly.
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
  learns what exists. It **filters out `shell::STORE_APP`**: that entry is how
  the shell finds home, and a start page listing itself offers a card that takes
  you where you already are. Filtered host-side so no guest has to hardcode the
  host's name for home; `--list` and the address bar still see the whole
  registry, because those are the registry and not the home screen.
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
- **Loading happens on another thread.** `Shell::start` spawns one that
  classifies the address, fetches it, compiles the component *and* instantiates
  it, then sends a `Landed` back down an `mpsc` channel. `poll` (once per frame,
  from the window backend) takes it if it has arrived; `wait` blocks for it, and
  is what the CLI and the headless runner use, since they have no frames to draw
  meanwhile. `open` is `start` + `wait`.
- `Pending::origin` — `User`, `Guest` or `Home`. Who asked changes what happens
  when the answer lands, and the answer lands long after the asking, so it has
  to be carried: `Home` falls back to the built-in list on failure and says
  nothing on success, `Guest` sets the `navigated` flag, `User` does neither.
- `Shell::open` — the single blocking navigation entry point: classify with
  `resolve`, then dispatch on the answer. Failure leaves the previous view
  intact and records the error in `status`; a bad link must not take the shell
  down. The view is only replaced once the destination is known good — which
  now means *fully built*, since `arrive` receives a live `MiniApp` rather than
  bytes to compile.
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

- **Chrome** — `chrome` draws the Home button, the address bar, the title and
  the status line; `home` draws the registry links. Host UI. The guest cannot
  draw it, reach it, or know it exists. The address bar lives in the chrome
  rather than on the home screen because it is the entry point for both modules
  — you must be able to retype an address from inside an app.
  **It returns `bool`, and on the start page it returns `false` having drawn
  nothing** (see §4.7): the caller skips the separator, because a separator
  under no chrome is a line across the top of the page. It keys on
  `Shell::is_start_page`, not on `View::Home` — the built-in fallback list is
  host UI and keeps its bar, or a store that failed to load would leave the
  shell with nowhere to type at all. An error status is the one thing that
  still draws there, since that page has nowhere else to put it.
- `navigate` is the one place a `Nav` becomes a `Shell::open` call, shared by
  the address bar and the home-screen links, so both accept the same input.
- `Body::of` picks the renderer for the area below the chrome. It reads the
  discriminant into a local first: the arms take `&mut self`, so the match
  cannot hold a borrow of `shell.view` across them.
- **Page** — `page` walks the guest's `Vec<UiCmd>` and records clicks into
  `pending_clicks` for the *next* frame. It is a `while` loop rather than a
  `for`, because a **run of consecutive `Tile`s is taken as one unit**:
  `tile_grid` chunks it into rows and centres each row individually (including
  a short last row, which is the row a wrapping layout leaves hanging), and the
  free function `tile` paints one card. Cards are painted, not composed out of
  a button, because the whole card is the click target and egui's button is not
  a two-line container.
- `install_theme` — rounder widgets and roomier spacing, applied through
  `all_styles_mut` so both the light and dark styles get it. Style only: layout
  stays a per-widget decision in `page`, because layout is what another backend
  would have to reimplement and a theme is what it can ignore. It also turns
  **`interaction.selectable_labels` off**: egui makes every label selectable by
  default, which drags a text caret across all of a mini-app's UI. A guest's
  `ui-label` is a rendering, not a document. Tiles take the other half of that
  — `on_hover_cursor(PointingHand)`, because the whole card is the link.

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

### `run-tests.sh` — the suite
36 assertions driven through the real binary: the guest cycle, the registry, the
store, every branch of address classification, the scheme guess, served
registries, and all three feature sets building. No `cargo test`, because what
needs proving is the boundary and the routing, and `--script` / `--input` make
both verifiable without a display.

Network cases use a throwaway `python3 -m http.server` and are **skipped**, never
failed, when it cannot start — a suite that reports failures for something it
never ran sends people hunting for bugs that are not there.

Three helpers, and the distinction is load-bearing: `check` (exit zero **and**
the text appears), `check_absent` (exit zero and the text does *not*), and
`check_fails` (non-zero, with the reason named). `check` did not always test the
status, which made the three build checks — whose expected substring is `""` —
match anything and pass over a failing build. An assertion that cannot fail is
worse than no assertion, because it is counted.

### `tools/componentize/` — core module → component
Wraps `wit_component::ComponentEncoder`. Does what `wasm-tools component new`
does, so the repo needs no external CLI on PATH.

### `mini-apps/store/` — the start page, as a guest
The catalogue, drawn the way a browser's new-tab page is: a `ui-heading` wordmark,
one `ui-search-field`, and a grid of `ui-tile` cards for whatever `list-apps`
returned; picking one calls `open-app`. No privileges the counter lacks.
Its existence is the argument: the host no longer owns a home screen, it owns a
*fallback*, and the start page is now a component that can be replaced without
touching the shell.

### `mini-apps/` — the other guests
`counter` (Rust, state in an `AtomicI32`), `hello` (Rust, second app to prove
hot-swap), `counter-cs` (C#, componentize-dotnet), `counter-py` (Python,
componentize-py — a whole CPython in the component, and `datetime` really does
run in the sandbox). Same WIT, no host change, for all of them.

The Rust ones are `cdylib` crates excluded from the workspace — they target
`wasm32-unknown-unknown` and import host functions that only exist at runtime,
so they must not be built for the native host target. The other two are built by
their own language's toolchain and are **optional**: `build-mini-apps.sh` skips
each cleanly when `dotnet` / `componentize-py` is absent, and the suite skips
rather than fails the assertions that need them.

One practical note for the Python one: `pip` puts its launcher in the
interpreter's scripts directory, which on Windows is routinely not on `PATH`, so
the build script asks Python where it is (`sysconfig.get_path('scripts')`)
before concluding it is not installed.

The size spread is the argument for mixing them: ~19 KB (Rust), ~2.2 MB (C#),
~18 MB (Python) — same interface, three amounts of runtime travelling with it.

---

## 4. Findings, and what became of them

Five things the code turned out to require that reading the plan alone would not
have told you. Each is kept with its outcome, because the reasoning is what
transfers — the next widget, the next capability, the next platform backend will
hit the same walls.

### 4.1 Content-type routing — built
`resolve.rs` and `View::WebPage` are this note, implemented, and
`registry::resolve` now makes the scheme guess that was deliberately deferred
out of it. The guess is the **last** step, after a registry-name lookup and an
existence check, so it can never shadow something real; and `looks_like_host`
inspects only the authority, so `app.wasm` stays a filename that happens to
contain a dot rather than becoming a DNS failure.

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

### 4.5 A trapped guest used to trap forever
`Shell::frame` returned the guest's error and left the app loaded, so the next
frame ran it again — sixty identical errors a second, with no way out but
closing the window. It now drops the app and falls back.

Deliberately to `View::Home`, the built-in list, and **not** through `go_home`:
if the guest that just trapped *was* the store, going home would reload it and
trap again. Recovery paths must not route through the thing that failed.

### 4.6 A browser-shaped start page needed vocabulary, not CSS
The start page is a guest, and a guest that can only say `label`, `button`,
`text-edit` can only ever be a vertical list. Nothing about *styling* the host
would have changed that — the shape was missing from the contract, not from the
renderer.

Three functions closed the gap, and the shape of them is the point:
`ui-heading(text, level)`, `ui-search-field(text, placeholder)`,
`ui-tile(title, subtitle)`. None takes a coordinate, a size, or a colour. The
guest says *what a thing is*; the host decides that a level-1 heading is 44pt and
centred, that a search field caps at 560pt wide, and that tiles flow 196pt wide
into centred rows. A `ui-tile-at(x, y)` would have been easier and would have
welded the sandbox to egui forever.

The two index-space decisions are the other half. `Search` draws from the same
counter as `TextEdit` and `Tile` from the same counter as `Button`, so the store
could trade its buttons for cards **without a single change to the headless
scripts that drive it** — `--script "2:0"` still clicks the first thing that can
be clicked. A separate counter per widget type would have been more obvious and
would have quietly broken every test.

Cost, for the next widget: `wit/world.wit`, `ui.rs`, `host.rs`, `gui.rs`, the
headless printer in `main.rs`, and the mini-app — the same five-file bill §4.4
predicted, plus the C# app's bindings, which regenerate from the WIT with no C#
source change at all.

### 4.7 The start page ended up with no chrome
Giving the store a search box put two text fields on the same screen: the host's
address bar above, showing `mini-apps/store/store.component.wasm`, and the
store's own field below it. Both looked like the place to type and neither was
obviously it — and the bar was showing an internal path no user typed or wants,
the shell's equivalent of `chrome://newtab`.

So on the start page — and only there — the host draws no chrome at all, and
the store's field takes an address as readily as a search term. Three things
follow, all of them load-bearing:

- **The guest still cannot navigate.** It offers an `Open address` tile and
  calls `open-app`; the host resolves and dispatches, exactly as it does for the
  address bar. Invariant 2 holds — what moved is who *draws* the field, not who
  performs the navigation.
- **It is offered, never acted on.** There is no Enter key in the UI vocabulary,
  and a field whose contents are treated as a destination while you are still
  typing would open `e`, then `ex`, then `exa`.
- **The fallback list keeps its bar.** `chrome` keys on `is_start_page` (the
  store is loaded), not on `View::Home`. Hiding the bar on the fallback too
  would mean a store that fails to load leaves the shell with no way to type an
  address — the one state where you most need one.

`Shell::go_home` also clears the status it just set: "loaded 43777 bytes of wasm
from …" is the right thing to say about a navigation and the wrong thing to
leave sitting on a start page.

### 4.8 A spinner was not the fix — the UI thread was
The window froze while a `.wasm` loaded, and "add a loader" is the obvious
answer and the wrong one: the thread that would have drawn the spinner was the
thread that was blocked. Windows was painting **"(Not Responding)"** over the
title bar, which is the operating system saying the same thing.

Two unbounded waits sat inside a frame, and only fixing both helped:

1. **The fetch** — `resolve::classify` calls `ureq::get(...).call()` for a URL,
   which waits on somebody else's server for as long as it likes.
2. **The build** — `Component::from_binary` is Cranelift compiling a whole
   component. A 19 KB Rust mini-app is nothing; `counter-cs` carries a trimmed
   .NET runtime and is 2.2 MB, and that was the freeze that survived fixing the
   fetch alone.

Both now happen on the loader thread, which sends back a `Landed` — a
`MiniApp` already instantiated and past its own `init`, not bytes for the UI
thread to work on. Every type involved is `Send`, `Engine` is a cheap `Send +
Sync` handle, and the registry snapshot is cloned in, so this cost no design:
`arrive` just installs a value.

The nice consequence is that the **old page keeps drawing** while the new one
loads. That is what a browser does, and it is also what keeps §4.5's rule true
— the view is replaced only on success, so a failed navigation still leaves you
exactly where you were, and the spinner (`gui::loading_bar`) is drawn *over* a
page that is still live rather than over a blank loading screen.

### 4.9 Moving the work off the UI thread was not enough — it has to not repeat
Adding the Python mini-app made §4.8's fix look wrong again: opening `counter-py`
took the whole *machine* down, not just the window. Three facts multiplied
together, and each one is worth keeping:

1. **Compiling that component took 31 seconds.** 18 MB of CPython through
   Cranelift, from scratch, every single time it was opened.
2. **Nothing cancels a compile.** `start_as` replacing `Shell::pending` drops
   the receiver, which means only that nobody hears the answer — the thread
   keeps burning a core to completion.
3. **Every click started another one.** A card clicked four times while nothing
   visibly happened is four concurrent 31-second compiles.

So the loader thread did exactly what §4.8 asked of it, and the pile-up came
from the *rate* of requests, not their placement. Both halves are fixed:

- **`Engine` caches compiled components on disk** (`main::engine`, via
  `wasmtime::Cache`). 31 s cold, **1.6 s warm** — the cost is now paid once per
  component per machine instead of once per visit. A cache that cannot be set up
  logs a line and is skipped; it is an optimisation, not a dependency.
- **`start_as` refuses redundant work**: the address already in flight is a
  no-op however it was asked for, and a *guest* cannot start a navigation at all
  while one is pending. The address bar still can, because the user overrides a
  page and a page does not override the user.

The general lesson is the one to carry: making slow work asynchronous stops it
blocking, and does nothing at all to stop it being *started again*. Anything
that can be triggered by a click needs both.

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
