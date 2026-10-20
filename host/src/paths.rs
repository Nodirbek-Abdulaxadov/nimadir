//! Where the shell keeps things on disk — what you configure, what it stores,
//! what it can always rebuild — per platform.
//!
//! A checkout used to be the only home the shell had: the registry and the
//! components were read relative to the working directory. An installed app
//! has no checkout, so these are the places it reads and writes instead.

use std::path::PathBuf;
use std::sync::OnceLock;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Keep everything under one directory. Android calls this with the app's
/// private storage, since nothing else there says where an app may write.
/// Only the first call counts.
pub fn set_root(dir: PathBuf) {
    let _ = ROOT.set(dir);
}

/// `set_root`, or else `NIMADIR_HOME` — which is how the test suite keeps the
/// shell away from the real configuration of whoever runs it.
fn root() -> Option<PathBuf> {
    ROOT.get()
        .cloned()
        .or_else(|| std::env::var_os("NIMADIR_HOME").map(PathBuf::from))
}

/// What the user configures: an `apps.json` here replaces the default registry.
pub fn config_dir() -> Option<PathBuf> {
    root()
        .map(|r| r.join("config"))
        .or_else(|| dirs::config_dir().map(|d| d.join("nimadir")))
}

/// What the shell keeps: its log, and later per-app storage.
pub fn data_dir() -> Option<PathBuf> {
    root()
        .map(|r| r.join("data"))
        .or_else(|| dirs::data_local_dir().map(|d| d.join("nimadir")))
}

/// What the shell can always fetch or build again: downloaded components,
/// compiled code.
pub fn cache_dir() -> Option<PathBuf> {
    root()
        .map(|r| r.join("cache"))
        .or_else(|| dirs::cache_dir().map(|d| d.join("nimadir")))
}
