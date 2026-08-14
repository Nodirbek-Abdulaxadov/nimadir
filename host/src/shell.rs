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
//!
//! An address is not assumed to be a mini-app any more: `resolve` classifies it
//! first, and the shell dispatches on the answer. That is the seam where the
//! second kind of destination — an old web page — enters the design.

use anyhow::Result;
use wasmtime::Engine;

use crate::host::MiniApp;
use crate::registry::AppEntry;
use crate::resolve::{self, Target};
use crate::ui::{FrameInput, UiCmd};

/// The registry name of the app that serves as the start page.
pub const STORE_APP: &str = "store";

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
    /// An old-style web page. Routing reaches this view today and the backend
    /// draws a placeholder; the webview renders into it in the next stage.
    WebPage { title: String, url: String },
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
    /// Set when the last `frame` honoured a guest's `open-app`, so a backend
    /// can tell "the UI changed because the app changed" from "the UI changed
    /// because the app redrew". Read-and-clear.
    navigated: bool,
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
            navigated: false,
        }
    }

    /// Whether the last `frame` navigated on the guest's behalf.
    pub fn took_navigation(&mut self) -> bool {
        std::mem::take(&mut self.navigated)
    }

    pub fn is_home(&self) -> bool {
        matches!(self.view, View::Home)
    }

    pub fn is_web_page(&self) -> bool {
        matches!(self.view, View::WebPage { .. })
    }

    /// Navigate to an address: classify it, then go wherever it turned out to
    /// lead. A failure leaves you where you were with the error in `status` — a
    /// bad link must not take the shell down.
    pub fn open(&mut self, src: &str, title: &str) {
        match self.try_open(src, title) {
            Ok(status) => {
                self.status = status;
                self.status_is_error = false;
            }
            Err(e) => {
                self.status = format!("could not open {src}: {e}");
                self.status_is_error = true;
            }
        }
    }

    /// Returns the status line to show on success. The view is only replaced
    /// once the destination is known good, so a failed navigation cannot leave
    /// the shell pointing at a half-loaded page.
    fn try_open(&mut self, src: &str, title: &str) -> Result<String> {
        match resolve::classify(src)? {
            Target::WasmApp(wasm) => {
                let app = MiniApp::load(&self.engine, &wasm, self.apps.clone())?;
                let n = wasm.len();
                self.view = View::App {
                    title: title.to_string(),
                    src: src.to_string(),
                    app: Box::new(app),
                };
                Ok(format!("loaded {n} bytes of wasm from {src}"))
            }
            Target::WebPage => {
                self.view = View::WebPage {
                    title: title.to_string(),
                    url: src.to_string(),
                };
                // Honest in both builds: without the webview feature there is
                // nothing to render the page with, and saying so beats a status
                // line that claims success over a blank area.
                #[cfg(feature = "webview")]
                let status = format!("web page: {src}");
                #[cfg(not(feature = "webview"))]
                let status =
                    format!("{src} is a web page — build with --features webview to render it");
                Ok(status)
            }
        }
    }

    /// Back to the start page. Drops the running instance (see module docs).
    ///
    /// The **store mini-app is the start page** when the registry lists one:
    /// the shell's own home screen is a fallback, not the destination. That
    /// inversion is the point of the store — the start page is an app like any
    /// other, written against the same WIT, replaceable without touching the
    /// host. A missing or broken store falls back to the built-in list rather
    /// than leaving the shell with nowhere to go.
    pub fn go_home(&mut self) {
        self.view = View::Home;
        self.status.clear();
        self.status_is_error = false;

        let Some(store) = self.apps.iter().find(|e| e.name == STORE_APP).cloned() else {
            return;
        };
        self.open(&store.src, &store.name);
        if self.status_is_error {
            let why = std::mem::take(&mut self.status);
            self.view = View::Home;
            self.status = format!("{why} — showing the built-in list instead");
        }
    }

    /// Run one frame of the current mini-app, then honour any navigation it
    /// asked for. On the home screen and on a web page there is no guest to
    /// run, so this is empty.
    pub fn frame(&mut self, input: FrameInput) -> Result<Vec<UiCmd>> {
        let (cmds, requested) = match &mut self.view {
            View::App { app, .. } => match app.frame(input) {
                // Taken after the call, never during it: see `open-app`.
                Ok(cmds) => (cmds, app.take_pending_open()),
                Err(e) => {
                    // A trapped guest traps again on the next frame, and the
                    // one after that. Drop it rather than rendering the same
                    // error sixty times a second forever.
                    //
                    // Deliberately the built-in list and not `go_home`: if the
                    // guest that just trapped *was* the store, going home would
                    // reload it and trap again.
                    self.view = View::Home;
                    self.status = format!("the app stopped: {e}");
                    self.status_is_error = true;
                    return Err(e);
                }
            },
            View::Home | View::WebPage { .. } => (Vec::new(), None),
        };

        let Some(src) = requested else {
            return Ok(cmds);
        };

        // A bare name from the guest means the same thing it means in the
        // address bar, so the store can list apps by name and not care where
        // they live.
        let (src, title) = crate::registry::resolve(&src, &self.apps);
        self.open(&src, &title);
        self.navigated = true;
        // `cmds` describes the app that just asked to leave. Drawing it now
        // would paint a frame of an app the shell has already dropped.
        Ok(Vec::new())
    }

    /// Drain log lines from the current mini-app.
    pub fn take_logs(&mut self) -> Vec<String> {
        match &mut self.view {
            View::App { app, .. } => app.take_logs(),
            View::Home | View::WebPage { .. } => Vec::new(),
        }
    }
}
