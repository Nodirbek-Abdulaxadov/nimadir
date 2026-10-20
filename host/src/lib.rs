//! The shell as a library: everything the desktop binary (`main.rs` — the CLI,
//! the window, the headless harness) and the Android entry point (`android/`)
//! share. Each of those is a thin front door onto the same `Shell`; nothing
//! about loading, sandboxing or drawing a mini-app differs between them.

#[cfg(all(feature = "gui", target_os = "android"))]
mod android;
#[cfg(feature = "gui")]
pub mod gui;
pub mod host;
pub mod registry;
pub mod resolve;
pub mod shell;
pub mod ui;
#[cfg(feature = "webview")]
mod webview;

use anyhow::Result;

/// The Wasmtime engine, with compiled components cached on disk and guest
/// running time bounded.
///
/// Compiling is the expensive half of opening a mini-app, and it scales with
/// the runtime the guest brought: nothing for a 19 KB Rust component, seconds
/// for the 18 MB Python one. Caching means that cost is paid once per component
/// per machine instead of once per visit.
///
/// A cache that cannot be set up is not fatal — it is an optimisation, and a
/// shell that refuses to start because a directory is unwritable would be
/// trading a real feature for a missing one.
pub fn engine() -> Result<wasmtime::Engine> {
    let mut config = wasmtime::Config::new();
    match wasmtime::Cache::from_file(None) {
        Ok(cache) => {
            config.cache(Some(cache));
        }
        Err(e) => eprintln!("(no compilation cache: {e})"),
    }
    // Guest time limits (see `host::start_epoch_ticker`). Compiled code checks
    // the epoch only when this is on, so it has to be decided here, for every
    // component the engine will ever compile.
    config.epoch_interruption(true);
    // wasmtime re-exports its own `Error` type, distinct from ours.
    let engine = wasmtime::Engine::new(&config).map_err(|e| anyhow::anyhow!("{e}"))?;
    host::start_epoch_ticker(&engine);
    Ok(engine)
}
