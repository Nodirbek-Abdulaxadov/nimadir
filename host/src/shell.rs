//! Navigation: the browser-shaped layer that sits above a single mini-app.
//!
//! The host used to be "load one .wasm, run it forever". This adds the piece a
//! browser has and that had no home before: a **current page** you can leave and
//! come back to. `Shell` owns the registry, the view you are on, and the loaded
//! instance — and nothing about how any of it is drawn, exactly like `ui.rs`.
//! The egui backend and the headless runner both drive this same type.
//!
//! Navigating away *drops* the `MiniApp`, which drops its Wasmtime `Store` and
//! the guest's linear memory with it. So a mini-app keeps no state across a
//! visit: going home and reopening `counter` starts again at 0. That is the
//! sandbox doing its job, not a bug.

use std::collections::HashSet;

use anyhow::{Context, Result};
use wasmtime::Engine;

use crate::host::MiniApp;
use crate::registry::AppEntry;
use crate::ui::UiCmd;

/// Which "page" the shell is on.
///
/// `title`/`src` are chrome, so only the window backend reads them; the
/// headless runner never draws chrome.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub enum View {
    /// The start page: the list of mini-apps.
    Home,
    /// A loaded mini-app, with the address it came from.
    App {
        title: String,
        src: String,
        app: Box<MiniApp>,
    },
}

pub struct Shell {
    engine: Engine,
    /// The home screen's links.
    pub apps: Vec<AppEntry>,
    pub view: View,
    /// Last navigation result, shown in the chrome (like a status bar).
    pub status: String,
    /// True when `status` describes a failed load.
    pub status_is_error: bool,
}

// `is_home`/`go_home` are navigation, which only the window backend performs.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
impl Shell {
    pub fn new(engine: Engine, apps: Vec<AppEntry>) -> Self {
        Shell {
            engine,
            apps,
            view: View::Home,
            status: String::new(),
            status_is_error: false,
        }
    }

    pub fn is_home(&self) -> bool {
        matches!(self.view, View::Home)
    }

    /// Navigate to a mini-app: fetch the bytes, instantiate it, and make it the
    /// current view. A failure leaves you where you were with the error in
    /// `status` — a bad link must not take the shell down.
    pub fn open(&mut self, src: &str, title: &str) {
        match self.try_open(src, title) {
            Ok(n) => {
                self.status = format!("loaded {n} bytes of wasm from {src}");
                self.status_is_error = false;
            }
            Err(e) => {
                self.status = format!("could not open {src}: {e}");
                self.status_is_error = true;
            }
        }
    }

    fn try_open(&mut self, src: &str, title: &str) -> Result<usize> {
        let wasm = load_wasm(src)?;
        let app = MiniApp::load(&self.engine, &wasm)?;
        let n = wasm.len();
        self.view = View::App {
            title: title.to_string(),
            src: src.to_string(),
            app: Box::new(app),
        };
        Ok(n)
    }

    /// Back to the start page. Drops the running instance (see module docs).
    pub fn go_home(&mut self) {
        self.view = View::Home;
        self.status.clear();
        self.status_is_error = false;
    }

    /// Run one frame of the current mini-app. On the home screen there is no
    /// guest to run, so this is empty — the home UI is drawn by the host.
    pub fn frame(&mut self, clicks: HashSet<u32>) -> Result<Vec<UiCmd>> {
        match &mut self.view {
            View::App { app, .. } => app.frame(clicks),
            View::Home => Ok(Vec::new()),
        }
    }

    /// Drain log lines from the current mini-app.
    pub fn take_logs(&mut self) -> Vec<String> {
        match &mut self.view {
            View::App { app, .. } => app.take_logs(),
            View::Home => Vec::new(),
        }
    }
}

/// Fetch mini-app bytes from a URL (http/https) or a local file path. This is
/// the "browser-like" fetch: remote code, retrieved at runtime, run sandboxed.
pub fn load_wasm(source: &str) -> Result<Vec<u8>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        use std::io::Read;
        let resp = ureq::get(source)
            .call()
            .with_context(|| format!("GET {source}"))?;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read(source).with_context(|| format!("read file {source}"))
    }
}
