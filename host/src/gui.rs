//! Native window backend (egui via eframe). Enabled with `--features gui`.
//!
//! Two things are drawn here, and keeping them apart is the whole point:
//!
//! * **Chrome** — the home screen, the address bar, the Home button. Host UI.
//!   The guest cannot draw it, reach it, or know it exists.
//! * **Page** — whatever the loaded mini-app described this frame via `UiCmd`.
//!
//! That split is the browser analogy made literal: untrusted code paints inside
//! the page area only, and navigation always stays in the host's hands.
//!
//! Note: this renders with egui, NOT Makepad. Makepad was the intended primary
//! toolkit, but the mini-app boundary here is *immediate-mode* (the guest
//! re-describes its whole UI every frame), and egui is an immediate-mode
//! toolkit, so it maps 1:1 with no adapter. Makepad is retained-mode + a DSL,
//! which would need a translation layer. Both are pure-Rust, GPU-rendered, and
//! use no HTML/CSS/DOM/JS — so the "no web stack" goal holds either way. To
//! switch to Makepad later, implement the same loop against `Shell`; nothing
//! else changes.

use std::collections::HashSet;

use anyhow::Result;
use eframe::egui;

use crate::registry;
use crate::shell::{Shell, View};
use crate::ui::UiCmd;

/// Where a home-screen interaction wants to navigate.
enum Nav {
    /// A link from the registry — already a real source.
    Listed { src: String, title: String },
    /// Raw text from the address bar — still has to be resolved.
    Typed(String),
}

pub fn run(shell: Shell) -> Result<()> {
    // Must happen on this thread, before any webview is created. A failure is
    // deliberately *not* fatal: mini-apps do not need a webview, so letting
    // module 1 refuse to initialise and taking the whole shell with it would be
    // the tail wagging the dog. The reason is carried instead, and a page that
    // cannot be rendered says why in its own view.
    #[cfg(feature = "webview")]
    let web_unavailable = crate::webview::init().err().map(|e| e.to_string());

    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "wasm-shell",
        native_options,
        Box::new(move |_cc| {
            #[allow(unused_mut)]
            let mut app = ShellApp::new(shell);
            #[cfg(feature = "webview")]
            {
                app.web_unavailable = web_unavailable;
            }
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;
    Ok(())
}

struct ShellApp {
    shell: Shell,
    /// Buttons clicked during the previous egui frame, fed to the guest next.
    pending_clicks: HashSet<u32>,
    /// Address bar contents.
    address: String,
    /// The address the bar was last synced to, so navigation can refresh it
    /// without overwriting what the user is in the middle of typing.
    synced_src: String,
    /// Last title pushed to the OS window, so we only send it on change.
    last_title: String,
    /// The live webview, when a page is open. `None` at every other moment —
    /// leaving a page destroys it (see `webview::WebPane`).
    #[cfg(feature = "webview")]
    web: Option<crate::webview::WebPane>,
    /// Why this platform has no webview at all, if it has none. Set once at
    /// startup and never cleared — retrying cannot change the answer.
    #[cfg(feature = "webview")]
    web_unavailable: Option<String>,
    /// Why *this* page's webview could not be created, if it could not.
    /// Remembered so the failure is reported once rather than retried sixty
    /// times a second, and cleared on navigation so the next page may differ.
    #[cfg(feature = "webview")]
    web_error: Option<String>,
}

impl ShellApp {
    fn new(shell: Shell) -> Self {
        ShellApp {
            shell,
            pending_clicks: HashSet::new(),
            address: String::new(),
            synced_src: String::new(),
            last_title: String::new(),
            #[cfg(feature = "webview")]
            web: None,
            #[cfg(feature = "webview")]
            web_unavailable: None,
            #[cfg(feature = "webview")]
            web_error: None,
        }
    }
}

impl eframe::App for ShellApp {
    // eframe 0.35 hands us a `&mut Ui` directly (no need to open a CentralPanel).
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // On Linux the webview lives in GTK's loop, not this one; without this
        // a page loads and then freezes. No-op on the other platforms.
        #[cfg(feature = "webview")]
        crate::webview::pump();

        self.sync_window_title(ui);
        self.chrome(ui);
        ui.separator();

        // Read the discriminant first: the arms take `&mut self`, so the match
        // cannot hold a borrow of `self.shell.view` across them.
        match Body::of(&self.shell) {
            Body::Home => {
                self.close_web();
                self.home(ui);
            }
            Body::App => {
                self.close_web();
                self.page(ui);
            }
            Body::WebPage => self.web_page(ui, frame),
        }

        // Immediate-mode: keep redrawing so button clicks are picked up promptly.
        ui.ctx().request_repaint();
    }
}

/// Which renderer the area below the chrome needs this frame.
enum Body {
    Home,
    App,
    WebPage,
}

impl Body {
    fn of(shell: &Shell) -> Self {
        match &shell.view {
            View::Home => Body::Home,
            View::App { .. } => Body::App,
            View::WebPage { .. } => Body::WebPage,
        }
    }
}

impl ShellApp {
    /// Mirror the current view into the OS window title, like a browser tab.
    fn sync_window_title(&mut self, ui: &egui::Ui) {
        let want = match &self.shell.view {
            View::Home => "wasm-shell".to_string(),
            View::App { title, .. } | View::WebPage { title, .. } => {
                format!("wasm-shell — {title}")
            }
        };
        if want != self.last_title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.last_title = want;
        }
    }

    /// The bar above the page: Home button, address bar, current title, status.
    ///
    /// The address bar lives here rather than on the home screen because it is
    /// now the entry point for *both* modules — you must be able to type a new
    /// address without first going home, exactly like a browser.
    fn chrome(&mut self, ui: &mut egui::Ui) {
        let (title, src) = match &self.shell.view {
            View::Home => ("wasm-shell".to_string(), String::new()),
            View::App { title, src, .. } => (title.clone(), src.clone()),
            View::WebPage { title, url } => (title.clone(), url.clone()),
        };
        let at_home = self.shell.is_home();

        // Follow navigation, like a browser's bar does: whatever you arrived at
        // is what it shows. Only on an actual change, so typing is never eaten.
        if src != self.synced_src {
            self.address = src.clone();
            self.synced_src = src.clone();
        }

        let mut go_home = false;
        let mut nav: Option<Nav> = None;
        // Submitted with nothing typed — needs to say so, or the button looks dead.
        let mut submitted_empty = false;

        let mut addr = std::mem::take(&mut self.address);
        ui.horizontal(|ui| {
            if !at_home {
                if ui.button("Home").clicked() {
                    go_home = true;
                }
                ui.separator();
            }
            let resp = ui.add(
                egui::TextEdit::singleline(&mut addr)
                    .desired_width(520.0)
                    .hint_text("counter   |   path/to/app.wasm   |   https://example.com"),
            );
            let entered =
                resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Open").clicked() || entered {
                if addr.trim().is_empty() {
                    submitted_empty = true;
                } else {
                    nav = Some(Nav::Typed(addr.clone()));
                }
            }
        });
        self.address = addr;

        // Opened straight from the address bar, the title *is* the address, and
        // the bar above is already showing it.
        if !at_home && title != src {
            ui.horizontal(|ui| {
                ui.strong(title.as_str());
            });
        }

        if submitted_empty {
            self.shell.status =
                "type a .wasm path, a URL, or the name of a listed app".to_string();
            self.shell.status_is_error = true;
        }

        // Drawn in the chrome so a failed navigation is visible from inside a
        // mini-app too, not only on the home screen.
        if !self.shell.status.is_empty() {
            if self.shell.status_is_error {
                ui.colored_label(egui::Color32::RED, self.shell.status.as_str());
            } else {
                ui.weak(self.shell.status.as_str());
            }
        }

        if go_home {
            // Drop any clicks aimed at the app we just left.
            self.pending_clicks.clear();
            self.shell.go_home();
        }
        if let Some(n) = nav {
            self.navigate(n);
        }
    }

    /// Perform a navigation requested by the chrome or a home-screen link.
    fn navigate(&mut self, nav: Nav) {
        self.pending_clicks.clear();
        let (src, title) = match nav {
            Nav::Listed { src, title } => (src, title),
            // Typed text only becomes an address here, so the address bar and
            // the CLI accept exactly the same things.
            Nav::Typed(input) => registry::resolve(&input, &self.shell.apps),
        };
        self.shell.open(&src, &title);
    }

    /// The start page: the registry as links. The address bar that used to live
    /// here is permanent chrome now, so this is purely the bookmark list.
    fn home(&mut self, ui: &mut egui::Ui) {
        // Where this frame wants to navigate, if anywhere.
        let mut nav: Option<Nav> = None;

        ui.add_space(4.0);
        ui.label("Mini-apps — click one to load it into this window.");
        ui.add_space(8.0);

        // Cloned so the loop holds no borrow of `self.shell` while we record a
        // click that will mutate it right after.
        let apps = self.shell.apps.clone();
        if apps.is_empty() {
            ui.weak("No mini-apps listed. Add some to apps.list.");
        }
        for e in &apps {
            ui.horizontal(|ui| {
                // Plain name: egui's bundled fonts have no arrow/bullet glyphs,
                // and a missing one renders as a tofu box.
                if ui.link(e.name.as_str()).clicked() {
                    nav = Some(Nav::Listed {
                        src: e.src.clone(),
                        title: e.name.clone(),
                    });
                }
                if !e.description.is_empty() {
                    ui.weak(e.description.as_str());
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.small(e.src.as_str());
            });
            ui.add_space(6.0);
        }

        if let Some(n) = nav {
            self.navigate(n);
        }
    }

    /// An old-style web page: module 1.
    ///
    /// With the `webview` feature the page is rendered by a native child
    /// webview covering exactly this area, and egui draws nothing here. Without
    /// it — or if the webview could not be created — this falls back to text
    /// that says where you are and why there is no page under it.
    fn web_page(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let View::WebPage { url, .. } = &self.shell.view else {
            return;
        };
        let url = url.clone();

        #[cfg(feature = "webview")]
        match self.show_web(frame, &url, ui) {
            // The webview owns this area now; anything egui painted into it
            // would be hidden behind a native surface anyway.
            Ok(()) => return,
            Err(e) => {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::RED, format!("no webview: {e}"));
            }
        }
        #[cfg(not(feature = "webview"))]
        let _ = frame;

        ui.add_space(8.0);
        ui.strong("Web page");
        ui.add_space(4.0);
        ui.label(url.as_str());
        ui.add_space(10.0);
        ui.weak("This address was classified as a web page, not a WASM component,");
        ui.weak("so the shell routed it away from the mini-app path.");
        #[cfg(not(feature = "webview"))]
        {
            ui.add_space(6.0);
            ui.weak("Rebuild with `--features webview` to render it here.");
        }
    }

    /// Put the webview over the page area, creating it on first use.
    ///
    /// The bounds are recomputed every frame from the space egui has left, so
    /// the page follows a window resize without any resize plumbing of its own.
    #[cfg(feature = "webview")]
    fn show_web(
        &mut self,
        frame: &eframe::Frame,
        url: &str,
        ui: &egui::Ui,
    ) -> anyhow::Result<()> {
        // A webview that cannot exist here will not start existing on the next
        // frame; report the reason instead of thrashing.
        if let Some(reason) = self.web_unavailable.as_ref().or(self.web_error.as_ref()) {
            anyhow::bail!("{reason}");
        }

        let rect = ui.available_rect_before_wrap();
        let ppp = ui.ctx().pixels_per_point();
        // egui works in points; the child surface is placed in physical pixels,
        // and the two only coincide when the user has not zoomed.
        let px = |v: f32| (v * ppp).round();
        let bounds = crate::webview::Bounds {
            x: px(rect.min.x) as i32,
            y: px(rect.min.y) as i32,
            w: px(rect.width()).max(1.0) as u32,
            h: px(rect.height()).max(1.0) as u32,
        };

        match &mut self.web {
            Some(pane) => pane.update(url, bounds),
            None => match crate::webview::WebPane::new(frame, url, bounds) {
                Ok(pane) => {
                    self.web = Some(pane);
                    Ok(())
                }
                Err(e) => {
                    self.web_error = Some(e.to_string());
                    Err(e)
                }
            },
        }
    }

    /// Leave the web page. Dropping the pane destroys the native surface, so a
    /// page you navigated away from stops running — the same rule mini-apps
    /// follow when the shell drops their `Store`.
    #[cfg(feature = "webview")]
    fn close_web(&mut self) {
        self.web = None;
        self.web_error = None;
    }

    #[cfg(not(feature = "webview"))]
    fn close_web(&mut self) {}

    /// The page area: purely whatever the guest described this frame.
    fn page(&mut self, ui: &mut egui::Ui) {
        let clicks = std::mem::take(&mut self.pending_clicks);

        let cmds = match self.shell.frame(clicks) {
            Ok(c) => c,
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("guest error: {e}"));
                return;
            }
        };

        // Translate the guest's UI commands into native egui widgets.
        for c in &cmds {
            match c {
                UiCmd::Label(t) => {
                    ui.label(t);
                }
                UiCmd::Button { index, text } => {
                    if ui.button(text).clicked() {
                        self.pending_clicks.insert(*index);
                    }
                }
            }
        }

        for l in self.shell.take_logs() {
            eprintln!("[wasm log] {l}");
        }
    }
}
