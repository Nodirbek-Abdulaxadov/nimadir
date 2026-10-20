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

use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::Result;
use wasmtime::Engine;

use nimadir_manifest::{Inspected, SignedBy};

use crate::host::MiniApp;
use crate::registry::AppEntry;
use crate::resolve::{self, Target};
use crate::ui::{FrameInput, UiCmd};

/// The registry name of the app that serves as the start page.
pub const STORE_APP: &str = "store";

/// The most places `back` remembers. A guest can navigate once per frame, so
/// an unbounded list is memory a page could grow forever.
const HISTORY_LIMIT: usize = 100;

/// A place the shell has been, to go back to. Only the address: leaving an
/// app destroys it, so going back opens it afresh — a browser reloading a page,
/// not one restored from memory.
struct Visit {
    src: String,
    title: String,
}

/// Which "page" the shell is on.
///
/// `title`/`src` are chrome, so only the window backend reads them; the
/// headless runner never draws chrome.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub enum View {
    /// The start page: the list of mini-apps.
    Home,
    /// A loaded mini-app, with the address it came from and what it says
    /// about itself — its manifest, and who signed it (ADR-1).
    App {
        title: String,
        src: String,
        app: Box<MiniApp>,
        info: Inspected,
    },
    /// An old-style web page. Routing reaches this view today and the backend
    /// draws a placeholder; the webview renders into it in the next stage.
    WebPage { title: String, url: String },
}

/// A navigation that has been started and has not arrived yet.
///
/// The address is being classified — and, for a URL, downloaded — on another
/// thread. `view` is deliberately *not* touched while this is outstanding: the
/// page you were on keeps drawing until the new one is known good, which is
/// both what a browser does and what keeps "a failed navigation leaves you
/// where you were" true.
struct Pending {
    title: String,
    src: String,
    rx: Receiver<Result<Landed>>,
    origin: Origin,
}

/// What comes back off the loader thread: a destination that is **ready**, not
/// one that still needs work doing to it.
///
/// The mini-app is compiled and instantiated over there too, not just fetched.
/// Downloading was never the only unbounded wait — compiling `counter-cs`
/// means Cranelift chewing through 2.2 MB of component, and a window that
/// stops repainting for that long is a window Windows paints "(Not
/// Responding)" over.
enum Landed {
    App {
        app: Box<MiniApp>,
        bytes: usize,
        cached: bool,
        info: Inspected,
    },
    WebPage,
}

/// Who asked for a navigation. It changes what happens when it lands, which is
/// why it has to be carried across the wait rather than decided at the call.
enum Origin {
    /// The address bar, a home-screen link, or the CLI.
    User,
    /// A guest, through `open-app`.
    Guest,
    /// The start page, on the way home. A failure falls back to the built-in
    /// list, and success says nothing — you are simply home.
    Home,
    /// `back`. The one navigation that does not add the page it leaves to the
    /// history, or going back twice would return you where you started.
    Back,
}

pub struct Shell {
    engine: Engine,
    /// The home screen's links.
    pub apps: Vec<AppEntry>,
    pub view: View,
    /// The navigation in flight, if any.
    pending: Option<Pending>,
    /// Last navigation result, shown in the chrome (like a status bar).
    pub status: String,
    /// True when `status` describes a failed load.
    pub status_is_error: bool,
    /// Set when the last `frame` honoured a guest's `open-app`, so a backend
    /// can tell "the UI changed because the app changed" from "the UI changed
    /// because the app redrew". Read-and-clear.
    navigated: bool,
    /// Where `back` goes, most recent last.
    history: Vec<Visit>,
    /// A fresher registry on its way, fetched off the UI thread.
    registry_rx: Option<Receiver<Vec<AppEntry>>>,
    /// An app's request the user has not answered yet (`asking`).
    asking: Option<Ask>,
}

/// An app asking to open somewhere the registry does not list.
pub struct Ask {
    /// The app asking, by its title.
    pub from: String,
    pub address: String,
    title: String,
}

// `is_home`/`go_home` are navigation, which only the window backend performs.
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
impl Shell {
    pub fn new(engine: Engine, apps: Vec<AppEntry>) -> Self {
        Shell {
            engine,
            apps,
            view: View::Home,
            pending: None,
            status: String::new(),
            status_is_error: false,
            navigated: false,
            history: Vec::new(),
            registry_rx: None,
            asking: None,
        }
    }

    /// Where the shell is, as `(title, address)` — `None` on the built-in
    /// list, which is a fallback and not a place.
    pub fn location(&self) -> Option<(&str, &str)> {
        match &self.view {
            View::Home => None,
            View::App { title, src, .. } => Some((title, src)),
            View::WebPage { title, url } => Some((title, url)),
        }
    }

    /// Remember where the shell is before it goes somewhere else.
    fn record(&mut self) {
        let Some((title, src)) = self.location() else {
            return;
        };
        let visit = Visit {
            src: src.to_string(),
            title: title.to_string(),
        };
        if self.history.len() == HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.history.push(visit);
    }

    /// Whether `back` would go anywhere — anywhere but out of the shell.
    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty() || !(self.is_start_page() || self.is_home())
    }

    /// Go back to where the shell was before, opening it afresh.
    ///
    /// With no history, back from an app is the start page — an app opened
    /// straight from the command line still has somewhere to go back to. From
    /// the start page itself there is nowhere: that returns `false`, which a
    /// phone reads as "leave the app" and a desktop ignores.
    pub fn back(&mut self) -> bool {
        match self.history.pop() {
            Some(v) if v.title == STORE_APP => self.go_start(),
            Some(v) => self.start_as(&v.src, &v.title, Origin::Back),
            None if self.is_start_page() || self.is_home() => return false,
            None => self.go_start(),
        }
        true
    }

    /// What is being loaded right now, if anything. The window backend draws a
    /// spinner from this; nothing else needs to know.
    pub fn loading(&self) -> Option<&str> {
        self.pending.as_ref().map(|p| p.src.as_str())
    }

    /// Fetch the registry at `url` off the UI thread, and install it when it
    /// arrives — into the shell, and into the running app, whose next
    /// `list-apps` sees it. The window opens on what it already has (the
    /// saved copy, or the built-in list) instead of waiting on a network.
    pub fn refresh_registry(&mut self, url: String) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Some(apps) = crate::registry::fetch_hosted(&url) {
                let _ = tx.send(apps);
            }
        });
        self.registry_rx = Some(rx);
    }

    /// Block until a registry refresh has arrived or failed. For the CLI and
    /// the headless runner, which have no frames to draw meanwhile.
    pub fn wait_registry(&mut self) {
        if let Some(Ok(apps)) = self.registry_rx.take().map(|rx| rx.recv()) {
            self.install_registry(apps);
        }
    }

    /// Whether a registry refresh is still on its way.
    pub fn refreshing(&self) -> bool {
        self.registry_rx.is_some()
    }

    fn install_registry(&mut self, mut apps: Vec<AppEntry>) {
        crate::registry::ensure_start_page(&mut apps);
        crate::diag::line(&format!("registry refreshed: {} entries", apps.len()));
        if let View::App { app, .. } = &mut self.view {
            app.set_apps(apps.clone());
        }
        self.apps = apps;
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

    /// Whether what is loaded is the **start page** rather than somewhere you
    /// navigated to. It is an ordinary `View::App` — that is the whole point of
    /// the store — but the chrome has to treat it as home: there is nothing to
    /// go back to, and `mini-apps/store/store.component.wasm` is an
    /// implementation detail, no more an address to show than `chrome://newtab`
    /// is in a browser.
    pub fn is_start_page(&self) -> bool {
        matches!(&self.view, View::App { title, .. } if title == STORE_APP)
    }

    /// Navigate to an address and **block** until it lands.
    ///
    /// The CLI and the headless runner use this: they have no frames to draw
    /// while a download is in flight, so waiting is all they could do anyway.
    /// The window backend uses `start` + `poll` and keeps painting.
    pub fn open(&mut self, src: &str, title: &str) {
        self.start(src, title);
        self.wait();
    }

    /// Begin a navigation and return immediately.
    ///
    /// Classifying an address means **fetching** it when it is a URL, and that
    /// is an unbounded wait on somebody else's server. Doing it inline froze
    /// the window until it finished — no repaint, no spinner, not even a close
    /// button, because the thread that would have drawn them was inside
    /// `ureq`. It happens on its own thread now, and the shell keeps drawing
    /// the page you are still on.
    pub fn start(&mut self, src: &str, title: &str) {
        self.start_as(src, title, Origin::User);
    }

    fn start_as(&mut self, src: &str, title: &str, origin: Origin) {
        // Nothing can cancel a compile once it is running: dropping the
        // receiver only means nobody hears the answer, the Cranelift work
        // carries on. So a second request while one is in flight does not
        // replace work, it *adds* it — and clicking a card four times while its
        // 18 MB Python component compiled put four of those on the machine at
        // once, which is enough to take the whole desktop down with it.
        //
        // Two rules, both matching what a browser does:
        //   * the address already loading is a no-op, however it was asked for;
        //   * a **guest** cannot queue a navigation behind one in flight at all.
        //     The address bar still can — the user overrides, a page does not.
        if let Some(p) = &self.pending {
            if p.src == src || matches!(origin, Origin::Guest) {
                return;
            }
        }

        let (tx, rx) = mpsc::channel();
        let address = src.to_string();
        // `Engine` is a cheap handle and both `Send` and `Sync`; the registry
        // snapshot is cloned because the guest is handed one at instantiation.
        let engine = self.engine.clone();
        let apps = self.apps.clone();
        // A registry entry may pin what this address must hold.
        // A registry entry may pin what this address must hold, and who
        // must have signed it.
        let entry = self.apps.iter().find(|e| e.src == src);
        let pin = entry.and_then(|e| e.sha256.clone());
        let publisher = entry.and_then(|e| e.publisher.clone());
        // If a second navigation starts first, this receiver is dropped and the
        // send fails — which is the right outcome: the answer to a question
        // nobody is asking any more goes nowhere.
        std::thread::spawn(move || {
            let landed =
                resolve::classify_pinned(&address, pin.as_deref()).and_then(|f| match f.target {
                    Target::WasmApp(wasm) => {
                        // Before anything is compiled: a component whose
                        // signature does not hold never reaches Cranelift.
                        let info = vouch(&address, &wasm, publisher.as_deref())?;
                        // Storage that outlives the app is for one with an
                        // identity that asked for it (ADR-2).
                        let keep_as = info
                            .identity()
                            .filter(|_| info.manifest.as_ref().is_some_and(|m| m.wants("storage")));
                        MiniApp::load(&engine, &wasm, apps, keep_as.as_deref()).map(|app| {
                            Landed::App {
                                app: Box::new(app),
                                bytes: wasm.len(),
                                cached: f.cached,
                                info,
                            }
                        })
                    }
                    Target::WebPage => Ok(Landed::WebPage),
                });
            let _ = tx.send(landed);
        });
        self.pending = Some(Pending {
            title: title.to_string(),
            src: src.to_string(),
            rx,
            origin,
        });
    }

    /// Block until the navigation in flight lands. No-op if none is.
    pub fn wait(&mut self) {
        let Some(p) = self.pending.take() else {
            return;
        };
        let result =
            p.rx.recv()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the loader thread died")));
        self.finish(p, result);
    }

    /// Take the navigation if it has landed yet. Called once per frame by the
    /// window backend; cheap, and does nothing at all when nothing is loading.
    pub fn poll(&mut self) {
        match self.registry_rx.as_ref().map(|rx| rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => {}
            Some(Ok(apps)) => {
                self.registry_rx = None;
                self.install_registry(apps);
            }
            // Fetched nothing usable; what is showing stays.
            Some(Err(TryRecvError::Disconnected)) => self.registry_rx = None,
        }
        match self.pending.as_ref().map(|p| p.rx.try_recv()) {
            None | Some(Err(TryRecvError::Empty)) => {}
            Some(Ok(result)) => {
                let p = self.pending.take().expect("pending, just matched");
                self.finish(p, result);
            }
            Some(Err(TryRecvError::Disconnected)) => {
                let p = self.pending.take().expect("pending, just matched");
                self.finish(p, Err(anyhow::anyhow!("the loader thread died")));
            }
        }
    }

    fn finish(&mut self, p: Pending, result: Result<Landed>) {
        match result.and_then(|t| self.arrive(t, &p)) {
            Ok(status) => {
                // "loaded 43 196 bytes of wasm from …" is the right thing to
                // say about a navigation and the wrong thing to leave sitting
                // on a start page. Arriving home is not news.
                crate::diag::line(&status);
                self.status = if matches!(p.origin, Origin::Home) {
                    String::new()
                } else {
                    status
                };
                self.status_is_error = false;
                // Only a guest-initiated navigation is news to the backend: it
                // has widget state belonging to an app that no longer exists.
                // Its own clicks it already knows about.
                if matches!(p.origin, Origin::Guest) {
                    self.navigated = true;
                }
            }
            Err(e) => {
                self.status = format!("could not open {}: {e:#}", p.src);
                self.status_is_error = true;
                crate::diag::line(&self.status);
                if matches!(p.origin, Origin::Home) {
                    let why = std::mem::take(&mut self.status);
                    self.view = View::Home;
                    self.status = format!("{why} — showing the built-in list instead");
                }
            }
        }
    }

    /// Install a destination that turned out to be good, and say so.
    ///
    /// The view is only replaced here — once the app is compiled, instantiated
    /// and past its own `init` — so a failed navigation cannot leave the shell
    /// pointing at a half-loaded page. Nothing expensive happens in here; the
    /// work was done before this was ever called.
    fn arrive(&mut self, landed: Landed, p: &Pending) -> Result<String> {
        // Recorded here, on arrival, not when the navigation starts: one that
        // fails leaves you where you were, and there is nothing to go back to.
        if !matches!(p.origin, Origin::Back) {
            self.record();
        }
        // A question from the page being left is no longer anyone's to ask.
        self.asking = None;
        let (title, src) = (p.title.clone(), p.src.clone());
        match landed {
            Landed::App {
                app,
                bytes,
                cached,
                info,
            } => {
                let from_cache = if cached { " (from the cache)" } else { "" };
                let status = format!(
                    "loaded {bytes} bytes of wasm from {src}{from_cache}{}",
                    describe(&info)
                );
                self.view = View::App {
                    title,
                    src,
                    app,
                    info,
                };
                Ok(status)
            }
            Landed::WebPage => {
                self.view = View::WebPage {
                    title,
                    url: src.clone(),
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
    ///
    /// Home is a navigation like any other, so the page it leaves is
    /// remembered: back from the start page returns to it.
    pub fn go_home(&mut self) {
        self.record();
        self.go_start();
    }

    /// `go_home` without remembering where it came from — for `back`, and for
    /// starting up, where there is nowhere to remember.
    fn go_start(&mut self) {
        self.asking = None;
        self.view = View::Home;
        self.status.clear();
        self.status_is_error = false;

        let Some(store) = self.apps.iter().find(|e| e.name == STORE_APP).cloned() else {
            return;
        };
        // Started, not waited on: going home must not freeze the window either,
        // and the registry's store may itself live at a URL. The built-in list
        // is what shows in the meantime, which is also what shows if it fails.
        self.start_as(&store.src, &store.name, Origin::Home);
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
                    self.asking = None;
                    self.status = format!("the app stopped: {e:#}");
                    self.status_is_error = true;
                    crate::diag::line(&self.status);
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

        // Somewhere the registry lists, or anything the start page sends —
        // there the address is the user's own, typed into its field. Anywhere
        // else, a page does not get to take the user somewhere on its own say:
        // the shell asks, and the app keeps running meanwhile.
        let listed = self.apps.iter().any(|e| e.src == src);
        if !listed && !self.is_start_page() {
            if self.asking.is_none() {
                let from = self.location().map_or("this app", |(t, _)| t).to_string();
                crate::diag::line(&format!("{from} asks to open {src}"));
                self.asking = Some(Ask {
                    from,
                    address: src,
                    title,
                });
            }
            return Ok(cmds);
        }

        self.start_as(&src, &title, Origin::Guest);
        // `cmds` describes the app that just asked to leave. Drawing it now
        // would paint a frame of an app the shell has already dropped.
        Ok(Vec::new())
    }

    /// An app's request to open an address the registry does not list, until
    /// the user answers it.
    pub fn asking(&self) -> Option<&Ask> {
        self.asking.as_ref()
    }

    /// The user's answer: open it, as if they had typed it, or stay.
    pub fn answer(&mut self, open: bool) {
        if let Some(ask) = self.asking.take() {
            if open {
                self.start_as(&ask.address, &ask.title, Origin::User);
            }
        }
    }

    /// When the current mini-app wants its next frame, in milliseconds, if it
    /// asked during the last one (`request-frame`).
    pub fn take_frame_request(&mut self) -> Option<u32> {
        match &mut self.view {
            View::App { app, .. } => app.take_frame_request(),
            View::Home | View::WebPage { .. } => None,
        }
    }

    /// The running app's identity, `<publisher>/<name>` — `None` for an app
    /// that is unsigned or has no manifest, which runs as unverified.
    pub fn identity(&self) -> Option<String> {
        match &self.view {
            View::App { info, .. } => info.identity(),
            View::Home | View::WebPage { .. } => None,
        }
    }

    /// Drain log lines from the current mini-app.
    pub fn take_logs(&mut self) -> Vec<String> {
        match &mut self.view {
            View::App { app, .. } => app.take_logs(),
            View::Home | View::WebPage { .. } => Vec::new(),
        }
    }
}

/// Check what a component says about itself before it is compiled: a
/// malformed manifest or a signature that does not hold is refused, and so is
/// one signed by somebody other than the publisher its registry entry names.
fn vouch(src: &str, wasm: &[u8], publisher: Option<&str>) -> Result<Inspected> {
    let info = nimadir_manifest::inspect(wasm)
        .map_err(|e| anyhow::anyhow!("{src}: {e} — it was not run"))?;
    if let Some(want) = publisher {
        match &info.signed_by {
            SignedBy::Publisher(p) if p == want => {}
            SignedBy::Publisher(p) => anyhow::bail!(
                "{src} is listed as published by {want}, but was signed by {p} — it was not run"
            ),
            SignedBy::Nobody => anyhow::bail!(
                "{src} is listed as published by {want}, but is not signed — it was not run"
            ),
        }
    }
    Ok(info)
}

/// What a landed app said about itself, for the status line.
fn describe(info: &Inspected) -> String {
    let what = info
        .manifest
        .as_ref()
        .map(|m| format!("{} {}", m.name, m.version));
    match (&info.signed_by, what) {
        (SignedBy::Publisher(p), Some(what)) => format!(" — {what}, signed by {p}"),
        (SignedBy::Publisher(p), None) => format!(" — signed by {p}, no manifest"),
        (SignedBy::Nobody, Some(what)) => format!(" — {what}, unsigned"),
        (SignedBy::Nobody, None) => String::new(),
    }
}
