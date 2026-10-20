//! The shell as a library: everything the desktop binary (`main.rs` — the CLI,
//! the window, the headless harness) and the Android entry point (`android/`)
//! share. Each of those is a thin front door onto the same `Shell`; nothing
//! about loading, sandboxing or drawing a mini-app differs between them.

#[cfg(all(feature = "gui", target_os = "android"))]
mod android;
pub mod diag;
#[cfg(feature = "gui")]
pub mod gui;
pub mod host;
pub mod paths;
pub mod registry;
pub mod resolve;
pub mod settings;
pub mod shell;
mod storage;
pub mod ui;
#[cfg(feature = "webview")]
mod webview;

// Desktop-only for beta (ROADMAP S3-8): wry on Android needs a Java activity of
// its own, and this app is a NativeActivity with no Java at all. A web page on
// Android is routed as everywhere else and says it cannot be shown.
#[cfg(all(feature = "webview", target_os = "android"))]
compile_error!("the `webview` feature is desktop-only: build Android without it");

use anyhow::Result;

/// The WIT package this host implements, as `wit/world.wit` declares it —
/// read from the file, so it cannot drift from what `bindgen!` compiled.
pub fn wit_package() -> &'static str {
    include_str!("../../wit/world.wit")
        .lines()
        .find_map(|l| l.trim().strip_prefix("package "))
        .map_or("unknown", |p| p.trim_end_matches(';'))
}

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
    // In the shell's own cache directory rather than wasmtime's default, which
    // it finds through a config file Android does not have — there, every
    // open compiled from scratch.
    let mut cache = wasmtime::CacheConfig::new();
    if let Some(dir) = paths::cache_dir() {
        cache.with_directory(dir.join("compiled"));
    }
    match wasmtime::Cache::new(cache) {
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
