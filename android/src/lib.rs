//! The Android front door — the spike for ROADMAP `S1-3`.
//!
//! The same `Shell` and the same window code as the desktop. This file does
//! only what an installed app has to and a checkout does not: put the
//! mini-apps somewhere the shell can read them, and tell it where. On the
//! desktop both come from the working directory (`S2-1` and `S2-2` replace
//! that); an Android app has no working directory worth the name, so the
//! components are compiled into the library and unpacked into the app's
//! private storage at start.

#![cfg(target_os = "android")]

use std::time::Instant;

use anyhow::{anyhow, Result};
use host::gui::AndroidApp;
use host::registry::AppEntry;
use host::shell::Shell;

/// `(name, description, component)`. Built by `build-mini-apps.sh` first.
/// `runaway` is listed here, unlike in `apps.json`, so the limits can be
/// checked by hand on a device.
const APPS: &[(&str, &str, &[u8])] = &[
    (
        "store",
        "The start page.",
        include_bytes!("../../mini-apps/store/store.component.wasm"),
    ),
    (
        "counter",
        "A counter; the count lives inside the guest.",
        include_bytes!("../../mini-apps/counter/counter.component.wasm"),
    ),
    (
        "hello",
        "A second app, to show hot-swap with no host rebuild.",
        include_bytes!("../../mini-apps/hello/hello.component.wasm"),
    ),
    (
        "counter-cs",
        "The counter in C# — a .NET runtime inside the sandbox, on a phone.",
        include_bytes!("../../mini-apps/counter-cs/counter-cs.component.wasm"),
    ),
    (
        "runaway",
        "Test fixture: spins, eats memory, floods the shell. The shell must survive each.",
        include_bytes!("../../mini-apps/runaway/runaway.component.wasm"),
    ),
];

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
    host::paths::set_root(base.clone());
    host::diag::init();
    let dir = base.join("mini-apps");
    std::fs::create_dir_all(&dir)?;

    let mut apps = Vec::new();
    for (name, description, bytes) in APPS {
        let path = dir.join(format!("{name}.component.wasm"));
        std::fs::write(&path, bytes)?;
        apps.push(AppEntry {
            name: name.to_string(),
            src: path.to_string_lossy().into_owned(),
            description: description.to_string(),
        });
    }
    log::info!("unpacked {} mini-apps into {}", apps.len(), dir.display());

    let engine = host::engine()?;

    // Spike data: how long each component takes to compile and start on this
    // device, read with `adb logcat -s nimadir`. Off the UI thread, so the
    // shell comes up while it runs.
    let bench = engine.clone();
    std::thread::spawn(move || {
        for (name, _, bytes) in APPS {
            let started = Instant::now();
            let outcome = host::host::MiniApp::load(&bench, bytes, Vec::new()).map(|_| ());
            log::info!(
                "load {name}: {} KB in {:.0?} ({})",
                bytes.len() / 1024,
                started.elapsed(),
                match outcome {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("{e:#}"),
                }
            );
        }
    });

    host::gui::run_android(Shell::new(engine, apps), app)
}
