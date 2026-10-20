//! The Android front door onto the shell.
//!
//! The same `Shell`, the same window code and the same registry as the
//! desktop. All this does is what only Android needs: put the shell's config,
//! data and caches in the app's private storage, send its log to logcat, and
//! hand eframe the activity. The start page is the store built into the host
//! (`builtin:store`); apps come from the registry, as they do everywhere else.
//!
//! For a registry of your own on a debug build, place one where the shell
//! looks for it:
//!
//! ```text
//! adb shell run-as dev.nimadir.shell mkdir -p files/config
//! adb shell run-as dev.nimadir.shell sh -c 'cat > files/config/apps.json' < apps.json
//! ```

#![cfg(target_os = "android")]

use anyhow::{anyhow, Result};
use host::gui::AndroidApp;
use host::registry;
use host::shell::Shell;

#[no_mangle]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("nimadir"),
    );
    // Nothing reads stderr on Android; a panic that is not logged is a
    // silent close.
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));

    if let Err(e) = run(app) {
        log::error!("{e:#}");
    }
}

fn run(app: AndroidApp) -> Result<()> {
    let base = app
        .internal_data_path()
        .ok_or_else(|| anyhow!("no internal data path"))?;
    // Config, data and caches all live in the app's private storage.
    host::paths::set_root(base);
    host::diag::init();

    let (mut apps, label, refresh) = registry::load_default();
    registry::ensure_start_page(&mut apps);
    log::info!("registry: {label}");

    let mut shell = Shell::new(host::engine()?, apps);
    if let Some(url) = refresh {
        // In the background: the window opens on what the shell already has.
        shell.refresh_registry(url);
    }
    host::gui::run_android(shell, app)
}
