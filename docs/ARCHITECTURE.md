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
| Mini-apps build | `./build-mini-apps.sh` | OK — `counter` 18 986 B, `hello` 15 551 B (rustc 1.98.1; the figure moves with the toolchain, which is why the suite reads it off the file) |
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
| `example.com` | — | guessed as `https://example.com`, then classified like any URL (the suite checks the guess against `localhost:9`, offline) |

All of the headless cases below are `./run-tests.sh` — 47 assertions, exit
non-zero on failure, repeatable, cleaning up after itself, and needing no
internet: every network case talks to a server on `127.0.0.1`. It is the suite;
the tables are what it covers.

**The store** (`store.component.wasm`, 44 KB), headless:

| Command | Result |
|---|---|
| `-- store --frames 1` | hero heading, search field, 4 app tiles read through `list-apps` — 5 registry entries, the store itself not among them |
| `-- store --input "1:0=hell" --script "2:0" --frames 4` | filtered to 1 of 4, tile indices renumbered, click opened `hello` via `open-app` |
| `-- store --input "1:0=example.com" --frames 2` | offered an `Open address` tile for the typed text |
| `-- store --input "1:0=mini-apps/hello/hello.component.wasm" --script "2:0" --frames 3` | clicking that tile navigated through `open-app` |
| `-- store --input "1:0=nowhere/missing.wasm" --script "2:0" --frames 3` | the failed navigation is reported (`[nav failed] …`), the store keeps running |
| `-- --registry <url> mini-apps/store/store.component.wasm` | catalogue fetched over HTTP and listed (by path: a served registry that does not list `store` cannot resolve the name) |
| `-- --registry <unreachable> --list` | fell back to the built-in list, did not abort |

**Sandbox limits** (`runaway.component.wasm`, a fixture that misbehaves on
purpose — see §4.10):

| Command | Result |
|---|---|
| `-- mini-apps/runaway/runaway.component.wasm --script "1:0" --frames 3` | `loop {}` in `update`: stopped after ~1 s, "it did not finish a frame within 1s and was stopped", exit 1 |
| `-- mini-apps/runaway/runaway.component.wasm --script "1:1" --frames 3` | `memory.grow` without end: stopped at the cap, "it tried to use more than 512 MiB of memory and was stopped" |
| `-- mini-apps/runaway/runaway.component.wasm --script "1:2" --frames 3` | one 1 MiB label sent in a loop: stopped at the per-frame cap in ~50 ms, "it sent more than 65536 widgets or 16 MiB of text in one frame and was stopped" |
| `-- mini-apps/runaway/runaway.component.wasm --frames 3` | untouched when it behaves |
| `…/huge.wasm` (65 MiB, served) | refused before the body is read: "larger than 64 MiB" |

**Scale** — a registry of 500 apps, the store drawing every tile each frame,
`--timing` (the call into the guest and back, UI collected; no renderer):

| Build | Unfiltered (500 tiles) | Filtered (100 tiles) |
|---|---|---|
| release | median 244 µs, max 1.65 ms | median 199 µs, max 253 µs |
| debug | median 3.2 ms, max 9.2 ms | median 2.3 ms, max 6.9 ms |

About 1.5 % of a 60 Hz frame in release, on this desktop (2026-10-20). Not
measured: egui laying out 500 cards, and any of it on a phone — `S5-7`'s
budget is a phone's, so the retained-tree question stays open until then.

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

- **`counter-py` is skipped** — `componentize-py` is not installed. The build
  script skips it cleanly by design, and the suite skips its assertions; the
  other apps still build (`counter-cs` too, 2.2 MB, with the .NET 10 SDK).
  Nothing is broken.
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
  `open-app`, `request-frame`, `storage-get` / `-set` / `-remove` / `-keys`,
  `ui-begin` / `ui-end`, `ui-checkbox`, `ui-choice`, `ui-separator`,
  `ui-progress`, `ui-submitted`, `image-load`, `ui-image`, `fetch`,
  `fetch-poll`, `locale`, plus the `app-entry`, `http-request` and
  `http-response` records.
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

**Groups** (`Begin { index, kind }` … `End`, kinds in `ui::Container`: row,
column, section, scroll). The guest names the kind as a string, so a kind
added later degrades to a column on an older host instead of breaking the
WIT. The host keeps them balanced — it closes what a guest left open and
ignores a stray `ui-end` — and draws at most `MAX_NESTING` (32) levels,
flattening the rest, because `gui.rs` walks them recursively (`draw_run`).
**Index spaces**, which every headless script depends on: *taps* (button,
tile, checkbox) share one counter, *values* (text field, search field,
choice) another; a choice's value travels as the option's number in
`FrameInput::edits`. `submitted` carries Enter / a phone keyboard's action
key for text fields, read by `ui-submitted` for the field drawn last.

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
- `request_frame` records into `frame_after` the same way — the earliest
  request of the frame, never under `MIN_FRAME_MS` (16 ms) — and
  `MiniApp::take_frame_request` hands it to the shell after the call. It is
  how a guest that shows time passing gets frames without input (§4.11).
- `MiniApp::load` — compile, link (`host-api` + WASI 0.2), instantiate,
  call `init`. Installs the store's limiter and an epoch deadline of
  `INIT_BUDGET_TICKS` *before* instantiating: a component can run code while it
  is instantiated, and a store with no deadline traps at its first check.
  Before linking, `newer_than_us` compares what the component imports from
  `nimadir:shell` with `provided_functions` (read from the WIT file itself) and
  turns a gap into "this app needs a newer nimadir: it uses `…`" — the linker's
  own wording is accurate and tells nobody what to do. Functions only: an
  interface exports types too.
- `MiniApp::frame` — clear the buffer, set the input, set a deadline of
  `FRAME_BUDGET_TICKS`, `call_update`, return a clone of the collected commands.
- **The limits** (§4.10): `EPOCH_TICK` (10 ms) is the unit, `FRAME_BUDGET_TICKS`
  (~1 s per `update`) and `INIT_BUDGET_TICKS` (~10 s for instantiate + `init`)
  bound time, `MAX_GUEST_MEMORY` (512 MiB) bounds linear memory.
  `start_epoch_ticker` is the thread that moves the clock; `struct GuestLimits`
  is the `ResourceLimiter`, which traps over the cap and remembers that it did.
  `MAX_FRAME_BYTES` (16 MiB) and `MAX_FRAME_CMDS` (65 536) bound what one frame
  may *send* the host: every capability that keeps a guest string calls
  `HostState::spend` before it pushes, and `struct Sent` remembers an overrun
  the way `GuestLimits` remembers memory. That is why `bindgen!` is
  `imports: { default: trappable }` — each `Host` method returns
  `wasmtime::Result`, and an `Err` is a trap in the guest.
- `explain` — turns a guest failure into a sentence. The two limits get one
  naming the limit; anything else is a real crash and keeps its whole error
  chain, because the trap's wording is the only clue to what the guest did.
- `MiniApp::take_pending_open` — the navigation the guest asked for, taken once
  it is off the stack and dropping it is legal.

WASI is linked in for runtime-bearing guest languages (C#, Go); the Rust
mini-apps import none of it, so it is purely additive.

### `host/src/resolve.rs` — the router
Decides what an address *is*, before the shell tries to go there.

- `enum Target` — `WasmApp(Vec<u8>)` (bytes included) or `WebPage`.
- `classify` — local paths by extension, `.wasm` URLs by name, everything else
  by a single `GET` whose `Content-Type` answers the question. `builtin:store`
  is the store compiled into the binary: `host/build.rs` copies
  `mini-apps/store/store.component.wasm` into it when it exists at build time
  (`has_builtin_store` says whether it did — a fresh clone's first build
  carries none and builds exactly as before).
- The fetch is deliberate: the request a `HEAD` probe would save is the one the
  component path has to make anyway, so the body rides along with the answer and
  nothing is downloaded twice.
- `application/octet-stream` and missing types fall back to the WASM magic
  number, because plenty of static hosts mislabel `.wasm`.
- `http()` — the one HTTP client, used by the registry fetch too: 10 s to
  connect, 30 s per read. ureq's default has no read timeout, so a server that
  accepts and then says nothing used to hold a loader thread forever.
- `MAX_DOWNLOAD` (64 MiB) — refused up front when `Content-Length` says so,
  cut off at the cap when it does not. The body is held whole before compiling,
  so without a cap an address could stream until the machine ran out of memory.
- `require_tls` — plain `http://` is refused unless the host is loopback, for
  components and registries alike, and before any request is made. Android's
  cleartext block does not cover Rust's sockets, so this is the only guard
  there.
- `classify_pinned` — `classify` for an address the registry pins: the
  SHA-256 is checked before anything is compiled, and a pinned *remote*
  component is kept in `<cache>/components/<sha256>.wasm`, read back (and
  re-hashed) on the next open, so it works offline. The loader thread in
  `Shell::start_as` goes through this, finding the pin by `src`.

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
- **History.** `arrive` records the page it is leaving as a `Visit` — an
  address and a title, nothing more, since leaving destroyed the app — and
  `back` pops one and opens it afresh, as a browser reloads a page.
  `Origin::Back` is the one navigation that records nothing, or two Backs
  would land where you started. `go_home` records (Home is a navigation;
  Back from the start page returns to the app), and `go_start` is the same
  thing without recording, for `back` and for startup. With no history, back
  from an app is the start page and back from the start page is `false` —
  which a phone reads as "leave". Capped at `HISTORY_LIMIT`, because a guest
  can navigate once per frame.
- **Where an app may send you.** A guest's `open-app` to an address the
  registry lists — or anything from the start page, whose field the user
  typed into — navigates. Anywhere else becomes an `Ask`: the app keeps
  running, the chrome shows "<app> wants to open <address> [Open] [Stay]"
  (where no guest can draw or click), and `Shell::answer` acts on the user's
  choice as if they had typed it. Headless, `--confirm` answers yes.
- **Vouching** (`vouch`, on the loader thread, before `MiniApp::load`): a
  component whose manifest is malformed or whose signature does not hold is
  refused before Cranelift sees it, and so is one signed by anybody other than
  the `publisher` its registry entry names. `View::App` keeps the result
  (`info`), the status line says what it found (`describe`), and
  `Shell::identity` is what per-app state will be keyed by.
- Fetching itself now lives in `resolve.rs`, since classification and loading
  are the same request.

### `host/src/storage.rs` — what an app keeps (ADR-2)
`Storage` behind the `storage-*` capabilities: a map of strings, 1 MiB per app
(`STORAGE_QUOTA`). `Storage::open(identity)` reads
`<data>/apps/<publisher>/<name>/storage.json` for an app that is signed and
asks for `storage` in its manifest — the shell decides that on the loader
thread and passes it to `MiniApp::load` — and is memory-only otherwise.
`tick` runs after every frame: it writes out at most every 250 ms, and when a
write is waiting it asks for a frame so an idle app's last change is not held
until the next tap. `Drop` writes whatever is left when the app is left.

### `host/src/net.rs` — the network, for guests (ADR-3)
`Net`, one per app: the origins its manifest declared (`origin_of` normalises
`scheme://host[:port]`), the ones the user allowed — remembered in
`<data>/apps/<id>/grants.json` for a signed app — and the requests, each
`Waiting` (on the user), `Flying` (on a thread) or `Done`. `fetch` checks TLS,
the declaration, a refusal, 8 MiB and 8 in flight before anything leaves;
`poll` hands an answer over once. The agent follows **no redirects**: one
would take a request to an origin nobody checked, so a 3xx goes back to the
app. While a request is flying the app gets frames every 50 ms to poll from;
while one waits on the user it gets none — the answer arrives as input. The
question is drawn at the top of the chrome on every page
(`ShellApp::network_question`); headless, `--confirm` answers it and the
runner waits out requests between frames (`settle`).

### `host/src/images.rs` — what an app shows
`Images::load` decodes PNG or JPEG on the host, reading the size from the
header *before* any pixel is allocated — a few bytes can claim any size — and
refuses past `MAX_IMAGE_SIDE` (4096) or the app's `MAX_APP_PIXELS` (8 MP). A
`Picture` keeps its RGBA behind an `Arc`, since the frame's commands are cloned
every frame, and an `id` unique across apps, which `gui.rs` caches its
textures by (`ShellApp::textures`), dropping any not drawn in a frame.

### `host/src/settings.rs` — what the shell keeps on disk
`kept` (each app's storage, by identity, with its size), `clear` (one app's —
the identity checked before it becomes a path, so `../` cannot escape),
`cache_bytes` / `clear_cache`. The settings page (`View::Settings`, at
`shell::SETTINGS_PAGE`, `nimadir:settings`) draws them in the window —
`ShellApp::settings`, read once per visit — and `--kept`, `--clear` and
`--clear-cache` print them. The store links to the page; a guest can only ask
for it, never press its buttons.

### `host/src/registry.rs` — the bookmarks
- `struct AppEntry` — `name`, `src`, `description`, and an optional
  `sha256` pin (a fourth `|` field in `apps.list`). Knows nothing about WASM;
  it is an address book.
- `parse` / `parse_json` — the pipe format and the JSON catalogue. JSON is what
  a registry *server* would serve, so one parser reads a local file and a remote
  one; `load_from` takes a URL as readily as a path. Every failure falls back
  rather than propagating.
- `load_default` — the registry when none is named: `NIMADIR_REGISTRY`, then
  `apps.json` / `apps.list` in the working directory (a checkout), then
  `apps.json` in `paths::config_dir`, then the hosted registry, then `builtin`.
  The working directory used to be the only place looked, so a shell started
  anywhere else had nothing to open.
- **The hosted registry** (`hosted_url`: `HOSTED_REGISTRY`, the project's Pages
  site, or `NIMADIR_HOSTED_REGISTRY`; empty turns it off). The window never
  waits on it: `load_default` hands back the copy saved from the last fetch
  (`hosted_copy`) — or the built-in list — plus the URL, and
  `Shell::refresh_registry` fetches it on a thread. When it lands it replaces
  `Shell::apps` and the running app's snapshot (`MiniApp::set_apps`), so the
  store's next `list-apps` shows it. `--list` and the headless runner wait for
  it instead, having nothing to draw. `tools/publish_registry.py` builds what
  is served, every entry pinned by SHA-256; `.github/workflows/registry.yml`
  deploys it.
- `ensure_start_page` — a registry with no `store` gets the one built into the
  binary (`builtin:store`), so there is always a start page to go home to.
- `builtin` — only what works from anywhere: the built-in store. It used to
  list the samples by their checkout paths, dead links everywhere a fallback
  is needed. Entries are still *sources*, not classified targets; `resolve`
  runs after.
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
  The bar shows a listed app by its registry name, not its `src`, which on a
  phone is a path inside the app's private storage.
- `go_back` — the chrome's Back button and the `BrowserBack` key (Android's
  system Back, a mouse's back button) both land here. When `Shell::back`
  has nowhere to go, Android closes the viewport, which leaves the app; a
  desktop window ignores it.
- **Redraw policy**, at the end of `ui`: another frame only when the next one
  will differ — `redraw` (a click or an edit this frame that the guest hears
  about next frame), a load in flight, a webview page (its GTK loop is pumped
  from here), or the guest's `request-frame` — which waits while the window is
  minimized or occluded (or the app is in the background), since nobody would
  see the frame. Input wakes egui by itself. See
  §4.11 for what the old unconditional `request_repaint` cost.
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
- `install_fonts` — egui's fonts plus Noto Sans (`host/assets/fonts`, OFL) as
  the last fallback: none of egui's has U+02BB, the ʻ of Uzbek's oʻ and gʻ.
- `palette` — two `Palette`s, `LIGHT` and `DARK`; everything painted by hand
  reads `palette::of(ui)`. `install_theme` builds both themes' visuals from them
  (`visuals`) and follows the system, falling back to light where the system
  does not say (egui's own fallback is dark).
- `install_theme` — rounder widgets and roomier spacing, applied through
  `all_styles_mut` so both the light and dark styles get it. Style only: layout
  stays a per-widget decision in `page`, because layout is what another backend
  would have to reimplement and a theme is what it can ignore. It also turns
  **`interaction.selectable_labels` off**: egui makes every label selectable by
  default, which drags a text caret across all of a mini-app's UI. A guest's
  `ui-label` is a rendering, not a document. Tiles take the other half of that
  — `on_hover_cursor(PointingHand)`, because the whole card is the link.

- `safe_rect` — the rectangle everything, chrome included, is laid out in. On
  the desktop the whole window. On Android the window minus the system bars
  and any cutout, plus a 12 pt side gutter: since API 35 the window runs under
  the status bar, and laid out over all of it the Home button sat under the
  clock where taps go to the system. winit reports no insets, so
  `android::SafeArea` (`host/src/android.rs`) reads them from the activity's
  `WindowInsets` over JNI, once per window size.

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

### `host/src/lib.rs` — the shell as a library
The modules above, plus `engine`, behind one crate that two front doors share:
`main.rs` (desktop CLI, window, headless harness) and `android/`. Nothing about
loading, sandboxing or drawing a mini-app differs between them.

- `engine` — the one `wasmtime::Engine`: compilation cache on disk, and
  `epoch_interruption(true)` with its ticker started. Turning epochs on changes
  the compiled code, so the cache entries from before it are not reused — the
  first open of each component after this change compiles from scratch once.

### `host/src/main.rs` — CLI and headless harness
- `main` — hand-rolled arg parsing (`--list`, `--headless`, `--frames`,
  `--script`, `--input`, `--back`, `--registry`, plus one positional source). A
  positional argument means "navigate straight there", skipping the home screen.
- `--back "4,6"` presses Back before those frames and prints `[back] now at …`
  or `[back] nothing to go back to`; a frame whose guest asked for another
  prints `[next frame in N ms]`.
- `parse_script` — `"1:0,2:0"` → `{frame: {button indices}}`.
- `parse_input` — `"1:0=hell"` → at frame 1, text field 0 holds `hell`. Repeatable
  rather than comma-separated, so typed text may contain anything.
- `run_headless` — runs N frames, injects the scripted clicks, prints
  every `UiCmd` and guest log line, `[nav] …` after a guest navigation that
  landed and `[nav failed] …` after one that did not.

`run_headless` is the project's test suite. There is no `#[test]` anywhere; the
`--script` flag *is* how the boundary is verified, and it needs no display.

### `android/` — the Android front door
A `cdylib` whose `android_main` points `paths` at the app's private storage,
loads the registry exactly as the desktop does (`registry::load_default`: a
saved hosted copy or the built-in store, refreshed in the background), and
calls `gui::run_android` — the desktop window code, handed the `AndroidApp`.
Nothing is bundled any more: the store is the host's `builtin:store`, and
apps come from the registry. A workspace member left out of
`default-members`, so a plain `cargo build` is unchanged.

Packaged by the Gradle project beside it (`settings.gradle.kts`, `app/`),
which holds no code: the activity is the platform's `NativeActivity`
(`android:hasCode="false"`, and `android.builtInKotlin=false` so AGP 9 does
not add 2.5 MB of Kotlin nothing loads). `build.sh` runs `cargo ndk` into
`app/src/main/jniLibs`, then Gradle for a debug APK and a release AAB, the
release signed from `NIMADIR_KEYSTORE` when set. Gradle strips the library
(48 → 30 MB) using the NDK `build.sh` found. It replaced the spike's
`cargo-apk`, which could not target an API level past the NDK's list and
could not build an AAB.

On Android eframe is built without X11, Wayland and `accesskit`
(`host/Cargo.toml`), and `webview` is a compile error (desktop-only for beta).
Findings from the device: `docs/spikes/android.md`.

### `run-tests.sh` — the suite
47 assertions driven through the real binary: the guest cycle, the registry, the
store, every branch of address classification, the sandbox limits, the scheme
guess, served registries, the download cap, and all three feature sets building.
None of it reaches the internet: the scheme guess is observed against
`localhost:9`, where nothing listens, and the store's typed-address click opens
a local path. Two cases used `example.com` and failed on any machine behind a
proxy or offline, with nothing broken. No `cargo test`, because what
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

### `crates/manifest/` — what a component says about itself (ADR-1)
`nimadir-manifest`, shared by the host and `nimadir`. A component carries
two custom sections: `nimadir-manifest` (JSON: `name`, `version`,
`permissions`, …) and `nimadir-signature` (Ed25519 public key + signature over
every byte before it — so it must be the last section). `inspect` reads both
and verifies; `with_manifest` and `sign` write them. It walks only the
top-level sections by their sizes (`sections`), so it needs no wasm parser.
`publisher_id` is the first 16 bytes of the key's SHA-256 in base32; an app's
identity is `<publisher>/<name>`, and an unsigned app has none.

### `tools/nimadir/` — the app developer's tool (`nimadir-cli`)
`new` writes a Rust, C# or Python project with its own copy of the WIT
(`templates::WIT`, `include_str!` of `wit/world.wit` — so a project is only ever
as new as the tool that made it) and a `manifest.json`. `build` finds the
language by what is in the directory: Rust builds for `wasm32-wasip2`, which
emits a component with no `componentize` step (~50 KB against ~19 KB the
`componentize` way, a trade made for having one tool fewer); C# with `dotnet`;
Python with `componentize-py`. It then packs the manifest in, signing with
`--key`. `run` builds and execs the shell; `check` runs the shell headless on
the file, which is what reports "needs a newer nimadir". `keygen`, `pack` and
`inspect` are the signing side (ADR-1); a lost key is a lost identity.

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

### `mini-apps/runaway/` — a test fixture, not an app
Three buttons: one that never returns from `update`, one that grows its memory
until stopped, one that sends the same 1 MiB label until the output cap stops it.
`run-tests.sh` clicks each and checks the guest is stopped with the limit named.
Deliberately **not** in `apps.json` — nothing about it belongs on a start page —
and opened by path. Built by `build-mini-apps.sh` like the others.

### `mini-apps/future/` — a test fixture, not an app
Built against its own `wit/world.wit`: the real contract plus one function,
`from-the-future`, which no host has. It stands in for an app made for a newer
nimadir, so the suite can check that opening one says so. Keep its WIT in step
with the real one, plus that function.

### `mini-apps/fetcher/`, `mini-apps/fuzz/` — test fixtures
`fetcher` fetches whatever is typed into its field, so the suite can sign it
against its own server and walk every path of ADR-3. `fuzz` calls every
`host-api` function forty times a frame with the worst inputs it can think of —
empty, long and multi-byte strings, NaN, garbage for an image, nonsense for a
URL, request numbers nobody issued — from a fixed seed, so any failure
replays. The suite runs it 300 frames and wants it still drawing at the end.

### `mini-apps/rates/` — the first real app
Exchange rates from the Central Bank of Uzbekistan (`cbu.uz`) with a
converter, in Uzbek, Russian or English by `locale()`. It declares `cbu.uz`
and `storage` in `manifest.json` — `build-mini-apps.sh` packs any app's
manifest into its component, and signs it when `NIMADIR_SIGNING_KEY` names a
key — keeps the last rates for opening offline, and lays itself out with
groups, a field and a choice. The suite declines its network request, so it
never needs the internet.

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

Things the code turned out to require that reading the plan alone would not
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

### 4.10 A guest could take the shell down with it — now bounded
"Untrusted code in a sandbox" held for *access* — a guest could reach nothing
the WIT did not give it — but not for *resources*. Four ways in, all closed:

- **Time.** `loop {}` in `update` ran on the UI thread and froze the window for
  good: no repaint, no Home, nothing but killing the process. On Android the
  same guest would get the *shell* killed as "not responding". Now an epoch
  deadline stops it after ~1 s, and `init` after ~10 s — `init` runs on a
  loader thread that nothing could otherwise reclaim.
- **Memory.** Nothing capped linear memory below the 4 GiB a 32-bit memory can
  address, so the host's allocation failed rather than the guest's. Now 512 MiB,
  and going over is a trap rather than a quiet `-1` from `memory.grow`: guests
  told "no" mostly abort anyway, and stopping them is the one outcome that is
  the same in every language.
- **Downloads.** No size cap and no read timeout. Now 64 MiB and 30 s of silence.
- **Output.** The memory cap bounds what a guest *holds*, not what it hands
  over. Every `ui-label` copies its string into a `Vec` on the host, so one
  1 MiB buffer sent in a loop filled the host's memory for the whole second
  before the frame deadline fired — and the guest's own memory never grew.
  Now a frame may send 65 536 widgets or 16 MiB of text, and the check runs
  *before* the push, so the host never holds more than that.

Epoch interruption was chosen over fuel because it is cheap enough to leave on
for every frame: compiled code checks a counter at loop headers and function
entries, and one thread moves the counter. Fuel counts instructions and costs on
every one of them. The trade is precision — a deadline is good to a tick
(10 ms) — which is nothing next to a second.

The stop goes through §4.5's path: the shell drops the app and shows why. The
reason names the limit (`explain`), because "wasm trap: interrupt" tells the
person in front of the window nothing about what happened.

### 4.11 Immediate mode is not "redraw forever" — found on a phone
`ShellApp::ui` ended with an unconditional `request_repaint()`, so a click
would reach the guest promptly. On a desktop that read as nothing. On the
Android spike's phone it was 54–58 % of a core with the store sitting idle,
and **99 % in the background**, with no window at all — eframe looping to
clear surfaces that did not exist.

Immediate mode needs a frame only when the next frame will differ, and the
shell knows every reason one would: input (egui already wakes for it), a
click or edit this frame that the guest hears about next frame (`redraw`), a
load in flight, and a webview page whose GTK loop is pumped from the frame.
The one reason it cannot know is the guest's own — a clock, an animation —
so that became a capability, `request-frame(after-ms)`: the guest asks, each
frame, for the next. An app that asks for nothing costs nothing.

The general lesson: a loop that is free on a plugged-in desktop is a battery
on a phone, and invariant 8 means the phone's numbers are the ones that count.

---

## 5. Invariants worth not breaking

1. **The WIT is the contract.** Add to it; do not reshape what exists. Both
   sides regenerate from the same file.
2. **Chrome is host-only.** Navigation must never be reachable by guest code.
3. **Leaving an app destroys it.** Nothing of it stays in host memory. What it
   kept under its identity is on disk, held by the host and given back only to
   that identity (ADR-2); nothing else may outlive the `Store`.
4. **Headless is the default feature set.** The host must keep building and
   running with no system GUI libraries; that is what makes it verifiable in CI.
   Every new system dependency goes behind a feature flag.
5. **Renderer stays behind `UiCmd`.** A backend swap should touch `gui.rs` and
   nothing else.
6. **No web stack in the mini-app path.** HTML/CSS/JS has no place on the guest
   side of the boundary.
7. **A guest's time and memory are bounded.** Every call into a guest runs under
   an epoch deadline and the store's limiter (§4.10). A new entry point into the
   guest must set a deadline before it calls in; with epochs on, a store without
   one traps at once. And a new capability that *keeps* what the guest hands it
   charges `HostState::spend` first, or it reopens the output hole.
8. **Desktop and Android are equal targets.** Neither is a port of the other.
   A feature lands when it works on both, and nothing may depend on what only a
   desktop has — hover, a physical keyboard, a mouse wheel, a window that is
   never backgrounded — outside a feature flag. (The Android backend is not built
   yet; this is the rule it is built under.)
