//! The shell as a library: everything the desktop binary (`main.rs` — the CLI,
//! the window, the headless harness) and the Android entry point (`android/`)
//! share. Each of those is a thin front door onto the same `Shell`; nothing
//! about loading, sandboxing or drawing a mini-app differs between them.

#[cfg(all(feature = "gui", target_os = "android"))]
mod android;
pub mod diag;
pub mod dom;
#[cfg(feature = "gui")]
pub mod gui;
#[cfg(feature = "gui")]
mod gui_dom;
pub mod host;
#[cfg(feature = "gui")]
mod i18n;
pub mod images;
mod net;
pub mod paths;
#[cfg(all(feature = "gui", target_os = "linux"))]
mod portal;
pub mod registry;
pub mod resolve;
pub mod settings;
pub mod shell;
mod storage;
pub mod text;
pub mod ui;
pub mod update;
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

static LOCALE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Tell the shell the user's locale, where the platform says it some other
/// way than environment variables — Android, from its `Locale` (`android.rs`).
/// Only the first call counts.
pub fn set_locale(tag: String) {
    let _ = LOCALE.set(tag);
}

/// The user's time zone, IANA (`time-zone`, 1.2): `TZ` when it names one —
/// the suite sets it — else what the system says; "" when it says nothing.
pub fn time_zone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim_start_matches(':');
        if tz.contains('/') && !tz.starts_with('/') {
            return tz.to_string();
        }
    }
    iana_time_zone::get_timezone().unwrap_or_default()
}

/// Minutes east of UTC in the user's time zone, now (`utc-offset`, 1.2).
pub fn utc_offset() -> i32 {
    use chrono::Offset;
    chrono::Local::now().offset().fix().local_minus_utc() / 60
}

/// The user's language and region as BCP 47 — `uz_UZ.UTF-8` in `LANG` is
/// `uz-UZ`. What `set_locale` was told, else the variables a Unix system sets;
/// `en-US` when nothing says (Windows, `C`, `POSIX`).
pub fn locale() -> String {
    if let Some(tag) = LOCALE.get() {
        return tag.clone();
    }
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .map(|v| v.split(['.', '@']).next().unwrap_or("").replace('_', "-"))
        .filter(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_else(|| "en-US".to_string())
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
