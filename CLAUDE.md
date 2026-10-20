# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A native desktop shell (Rust + Wasmtime + egui) that loads sandboxed WASM
**component** mini-apps at runtime and renders their UI natively — no HTML/CSS/JS
on the mini-app path. `README.md` explains *why*; **`docs/ARCHITECTURE.md` is the
code map** — read it before changing anything non-trivial. It is kept current and
written against greppable symbol names. **`docs/ROADMAP.md`** is the plan from
MVP to beta — sprints, item IDs, and the exit criteria beta is held to.

## Build

```bash
rustup target add wasm32-unknown-unknown   # one-time
./build-mini-apps.sh                       # all mini-apps -> *.component.wasm
cargo build -p host                        # headless (default features)
cargo build -p host --features gui         # + native window (eframe/egui)
cargo build -p host --features webview     # + wry; needs WebView2 / libwebkit2gtk-4.1-dev
```

On Linux the webview is X11-only: `webview::check_parent` refuses a Wayland
surface with an error rather than letting wry panic, so run it under X11/XWayland.

Android (`android/`) is the Rust crate plus a Gradle project that only packages
it. Needs `ANDROID_HOME` with an NDK, JDK 17+, `rustup target add
aarch64-linux-android` and `cargo install cargo-ndk`; build the mini-apps first
(the host bakes the store in):

```bash
./android/build.sh            # cargo-ndk -> jniLibs, then debug APK + release AAB
./android/build.sh install    # ...and install the APK on the device adb sees
./tools/android-smoke.sh      # on a device: starts, idles cold, Back exits, no panic
```

The app is a NativeActivity with `android:hasCode="false"` — no Java or Kotlin.
Don't move it to GameActivity without first solving IME text: winit 0.30 drops
the text events GameActivity sends, which would break typing entirely.

`host` is a library (`lib.rs`) plus the desktop binary (`main.rs`); `android/`
is the second front door onto the same library.

`*.component.wasm` is gitignored — **a fresh clone has no mini-apps and the shell
has nothing to open until `build-mini-apps.sh` runs.** Build the mini-apps
*before* the host: `host/build.rs` bakes the store into the binary
(`builtin:store`) only if it exists at that moment.

Outside a checkout the registry comes from `NIMADIR_REGISTRY`, then
`<config>/apps.json`, then the hosted registry (GitHub Pages, fetched in the
background; `NIMADIR_HOSTED_REGISTRY` overrides it, empty disables it — the suite
disables it so it never needs the internet), then the built-in store alone. `NIMADIR_HOME=<dir>` moves
config, data (the log) and caches under one directory — the suite always sets it.

Rebuilding one Rust mini-app is two steps, because `wit-bindgen` emits a *core
module* that must then be encoded into a component:

```bash
cargo build --release --target wasm32-unknown-unknown --manifest-path mini-apps/<app>/Cargo.toml
./target/release/componentize mini-apps/<app>/target/wasm32-unknown-unknown/release/<app>.wasm \
                              mini-apps/<app>/<app>.component.wasm
```

The mini-apps are **excluded from the workspace** (`Cargo.toml`) and must never be
built for the host target — they import host functions that only exist at runtime.

`counter-cs` (C#) needs the .NET 10 SDK and is skipped cleanly when `dotnet` is
absent. Its csproj pulls the NativeAOT-LLVM cross-compiler for the *build host*
via `runtime.$(NETCoreSdkPortableRuntimeIdentifier).microsoft.dotnet.ilcompiler.llvm`;
hardcoding a RID there breaks the build on every other platform.

`counter-py` (Python) is built by `componentize-py` (`pip install componentize-py`)
straight from `wit/`, with no cargo step, and is likewise skipped when absent — as
are its assertions in the suite. It bundles CPython (~18 MB), so its **first open
compiles for ~30 s**; the engine's on-disk cache (`main::engine`) makes later
opens ~2 s.

Signing an app (ADR-1 — identity is `<publisher>/<name>`, publisher from the key):

```bash
./target/debug/nimadir-pack keygen my.key          # keep it secret, back it up
./target/debug/nimadir-pack pack in.wasm manifest.json out.wasm --key my.key
./target/debug/nimadir-pack inspect out.wasm
```

## Run

```bash
cargo run -p host --features gui                 # window, opens on the store
cargo run -p host --features gui -- counter      # skip the start page
cargo run -p host -- counter --script "1:0,2:0,3:0,5:1" --frames 8   # headless
cargo run -p host -- --list                      # the registry, as text
```

A binary built with `--features gui` opens a **window** unless you pass
`--headless`; the headless harness is only the default when the feature is off.

Headless flags: `--frames N`, `--script "frame:button,…"` (inject clicks),
`--input "frame:field=text"` (repeatable; set a text field), `--back "frame,…"`
(press Back before those frames), `--registry <path-or-url>`.

Frames count from 0 (default 8). Widget indices are handed out per frame in call
order, and each counter is shared by two widgets: `ui-button` + `ui-tile` for
clicks, `ui-text-edit` + `ui-search-field` for text. The headless output prints
the index (`button0`, `tile2`, `search0`) — read the one to script off a run.

Don't pipe the binary into `head`: it panics (exit 101) when stdout closes early.
Capture the output instead.

## Test

```bash
./run-tests.sh        # the whole suite
./run-tests.sh -v     # with each command's full output
```

There is **no `cargo test`** and no `#[test]` anywhere — the thing worth proving is
the host↔guest boundary and the address routing, and both are driven end to end
through the real binary with `--script` / `--input`, needing no display. To run a
single case, copy that `check` line's command and run it directly, e.g.:

```bash
./target/debug/host store --input "1:0=hell" --script "2:0" --frames 4
```

The suite rebuilds the host but **not** the mini-apps: it runs
`build-mini-apps.sh` only when `store` or `runaway` is missing. After editing a
guest, rebuild it first, or the suite tests the stale component.

Network cases need `python3` (throwaway HTTP server) and `curl`; without them they
are **skipped, never failed**. They talk only to `127.0.0.1` — the suite needs no
internet. Byte-count assertions read the size off the built file (`COUNTER_BYTES`),
so a toolchain change that resizes a component does not break them.

`mini-apps/runaway` and `mini-apps/future` are **test fixtures**, not apps:
`runaway` spins, eats memory and floods the shell on demand so the suite can check
the sandbox limits hold; `future` is built against its own copy of the WIT with one
extra function, standing in for an app newer than the host. Both stay out of
`apps.json`. A change to `wit/world.wit` should be mirrored into
`mini-apps/future/wit/world.wit` (plus its one extra function).

## Architecture in brief

- **`wit/world.wit` is the entire contract**, and the only place it is defined:
  `wit-bindgen` generates the guest side, `wasmtime::component::bindgen!` the host
  side, from the same file. Changing it changes both sides at once.
- The host is a **frame pump around a sandbox**: it hands the guest what the user
  did to the last frame, calls `update()`, and collects the guest's `ui-*` calls
  into a `Vec<UiCmd>` (`ui.rs`) that a backend then draws — egui (`gui.rs`) or
  `println!` (`main.rs`). Swapping renderers should touch `gui.rs` only. The
  window draws **only when something happens** (input, a load, a webview page);
  a guest that animates asks with `request-frame`. Never reintroduce an
  unconditional `request_repaint()` — it kept a phone's core busy in the
  background.
- `resolve.rs` **classifies an address before navigation** (component vs. web
  page); `shell.rs` owns `View` and dispatches on the answer. A failed load leaves
  the previous view intact.
- **The start page is a mini-app**, not host code: `mini-apps/store` reads the
  registry through `list-apps` and navigates with `open-app`, with no privileges
  the counter lacks. The host's built-in list is a *fallback*.
- `open-app` is a **request**, drained after `call_update` returns — a guest cannot
  be dropped from inside its own frame. A frame that navigated returns no commands.
- Leaving an app **destroys** it (`Store` + linear memory). Reopening starts fresh;
  that is the sandbox working, not lost state.
- A guest's **time and memory are bounded** (`host.rs`): an epoch deadline of ~1 s
  per `update` and ~10 s for instantiate + `init`, 512 MiB of linear memory,
  16 MiB / 65 536 widgets of output per frame, and 64 MiB / 30 s of silence per
  download (`resolve.rs`). A stuck guest is stopped, not waited on. Any new call
  into a guest must set a deadline first.

Adding a capability = a line in `wit/world.wit` + a method on `impl Host for
HostState` (`host.rs`). Every import is trappable, so the method returns
`wasmtime::Result`; one that keeps what the guest hands it must call
`HostState::spend` before storing it. Adding a widget also touches `ui.rs`, `gui.rs`, and the
headless printer — and any stateful widget must key its egui id off the
**guest-assigned index, not host draw order** (see ARCHITECTURE §4.4).

Adding a Rust mini-app: create the crate, add it to `exclude` in the workspace
`Cargo.toml`, its `target/` to `.gitignore`, and it to `APPS` in
`build-mini-apps.sh`. Then list it in both `apps.json` (which takes precedence)
and `apps.list` (kept in step by hand). The store assertions in `run-tests.sh`
count the registry (`"4 apps"`, `1 of 4 apps matching`), so a new entry means
updating them. The entry named `store` is special (`shell::STORE_APP`): Home
loads it, and `list-apps` filters it out.

## Invariants

The WIT is additive-only; chrome (address bar, Home) is host-only and never
reachable by guest code; **headless with no system GUI libraries stays the default
feature set** — every new system dependency goes behind a feature flag; no web
stack on the guest side of the boundary; and **desktop and Android are equal
targets** — a feature lands when it works on both, and nothing may rely on what
only a desktop has (hover, a physical keyboard) outside a feature flag. Full list:
`docs/ARCHITECTURE.md` §5.
