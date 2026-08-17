# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A native desktop shell (Rust + Wasmtime + egui) that loads sandboxed WASM
**component** mini-apps at runtime and renders their UI natively — no HTML/CSS/JS
on the mini-app path. `README.md` explains *why*; **`docs/ARCHITECTURE.md` is the
code map** — read it before changing anything non-trivial. It is kept current and
written against greppable symbol names.

## Build

```bash
rustup target add wasm32-unknown-unknown   # one-time
./build-mini-apps.sh                       # all mini-apps -> *.component.wasm
cargo build -p host                        # headless (default features)
cargo build -p host --features gui         # + native window (eframe/egui)
cargo build -p host --features webview     # + wry; needs WebView2 / libwebkit2gtk-4.1-dev
```

`*.component.wasm` is gitignored — **a fresh clone has no mini-apps and the shell
has nothing to open until `build-mini-apps.sh` runs.**

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
`--input "frame:field=text"` (repeatable; set a text field), `--registry <path-or-url>`.

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

Network cases need `python3` (throwaway HTTP server) and `curl`; without them they
are **skipped, never failed**. Several assertions match exact byte counts
(`"loaded 17643 bytes"`) — a different rustc produces a different-sized component,
so those fail after a toolchain change with nothing actually broken; rebuild the
mini-apps and update the numbers together.

## Architecture in six lines

- **`wit/world.wit` is the entire contract**, and the only place it is defined:
  `wit-bindgen` generates the guest side, `wasmtime::component::bindgen!` the host
  side, from the same file. Changing it changes both sides at once.
- The host is a **frame pump around a sandbox**: it hands the guest what the user
  did to the last frame, calls `update()`, and collects the guest's `ui-*` calls
  into a `Vec<UiCmd>` (`ui.rs`) that a backend then draws — egui (`gui.rs`) or
  `println!` (`main.rs`). Swapping renderers should touch `gui.rs` only.
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

Adding a capability = a line in `wit/world.wit` + a method on `impl Host for
HostState` (`host.rs`). Adding a widget also touches `ui.rs`, `gui.rs`, and the
headless printer — and any stateful widget must key its egui id off the
**guest-assigned index, not host draw order** (see ARCHITECTURE §4.4).

Adding a mini-app: create the crate, add it to `exclude` in the workspace
`Cargo.toml`, to `APPS` in `build-mini-apps.sh`, and to `apps.json` (which takes
precedence over the hand-editable `apps.list`).

## Invariants

The WIT is additive-only; chrome (address bar, Home) is host-only and never
reachable by guest code; **headless with no system GUI libraries stays the default
feature set** — every new system dependency goes behind a feature flag; and no
web stack on the guest side of the boundary. Full list: `docs/ARCHITECTURE.md` §5.
