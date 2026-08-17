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

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use eframe::egui;

use crate::registry;
use crate::shell::{Shell, View};
use crate::ui::{FrameInput, UiCmd};

/// Where a home-screen interaction wants to navigate.
enum Nav {
    /// A link from the registry — already a real source.
    Listed { src: String, title: String },
    /// Raw text from the address bar — still has to be resolved.
    Typed(String),
}

pub fn run(mut shell: Shell) -> Result<()> {
    // The window's start page is the store mini-app, not the shell's built-in
    // list — `go_home` decides which, and falls back if the store is missing.
    // Only when the CLI did not already navigate somewhere.
    if shell.is_home() {
        shell.go_home();
    }

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
        Box::new(move |cc| {
            install_theme(&cc.egui_ctx);
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

/// The palette: a browser's, not a terminal's. White surfaces, one blue
/// accent, grey borders, text dark enough to read on all of it.
mod palette {
    use eframe::egui::Color32;

    pub const SURFACE: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
    /// Cards and unpressed controls — the faint grey Chrome uses for chips.
    pub const CARD: Color32 = Color32::from_rgb(0xf1, 0xf3, 0xf4);
    pub const BORDER: Color32 = Color32::from_rgb(0xda, 0xdc, 0xe0);
    pub const TEXT: Color32 = Color32::from_rgb(0x20, 0x21, 0x24);
    pub const MUTED: Color32 = Color32::from_rgb(0x5f, 0x63, 0x68);
    pub const ACCENT: Color32 = Color32::from_rgb(0x1a, 0x73, 0xe8);
    /// The blue wash under a hovered control.
    pub const ACCENT_WASH: Color32 = Color32::from_rgb(0xe8, 0xf0, 0xfe);
    pub const DANGER: Color32 = Color32::from_rgb(0xd9, 0x30, 0x25);

    /// One per app icon. Colour is decoration, but it is also the fastest way
    /// to tell two cards apart before you have read either of them.
    const BADGES: [Color32; 8] = [
        Color32::from_rgb(0x1a, 0x73, 0xe8), // blue
        Color32::from_rgb(0xea, 0x43, 0x35), // red
        Color32::from_rgb(0x34, 0xa8, 0x53), // green
        Color32::from_rgb(0xf9, 0xab, 0x00), // yellow
        Color32::from_rgb(0xa1, 0x42, 0xf4), // purple
        Color32::from_rgb(0x12, 0xb5, 0xcb), // teal
        Color32::from_rgb(0xfa, 0x7b, 0x17), // orange
        Color32::from_rgb(0xe5, 0x25, 0x92), // pink
    ];

    /// Stable per-title, so an app keeps its colour across frames, restarts and
    /// filtering — a badge that changed colour as you typed would be noise.
    pub fn badge(title: &str) -> Color32 {
        let hash = title
            .bytes()
            .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        BADGES[hash as usize % BADGES.len()]
    }
}

/// The window's visual defaults: a light, browser-shaped palette, rounder
/// widgets, more air between them. Applied once at startup — egui re-reads the
/// style every frame, so no other code has to know this happened.
///
/// It is deliberately *style* and not layout. Layout stays a per-widget
/// decision in `page`, because that is the part a different backend would have
/// to reimplement, and a theme it can simply ignore.
///
/// The theme is **pinned to light**, and both stored styles get the same
/// visuals, so an OS that says "dark" cannot hand back the near-black default
/// this replaced.
fn install_theme(ctx: &egui::Context) {
    use palette::*;

    ctx.set_theme(egui::ThemePreference::Light);

    let mut v = egui::Visuals::light();
    v.panel_fill = SURFACE;
    v.window_fill = SURFACE;
    v.faint_bg_color = CARD;
    v.extreme_bg_color = SURFACE;
    v.hyperlink_color = ACCENT;
    v.error_fg_color = DANGER;
    v.warn_fg_color = DANGER;
    v.window_stroke = egui::Stroke::new(1.0, BORDER);
    v.selection.bg_fill = ACCENT_WASH;
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT);

    w.inactive.bg_fill = CARD;
    w.inactive.weak_bg_fill = CARD;
    w.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);

    w.hovered.bg_fill = ACCENT_WASH;
    w.hovered.weak_bg_fill = ACCENT_WASH;
    w.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    w.hovered.fg_stroke = egui::Stroke::new(1.0, ACCENT);

    // `active` is also where `strong_text_color()` comes from, so its
    // foreground has to stay legible on a light background — a solid blue
    // button with white text here would turn every bold label white on white.
    w.active.bg_fill = ACCENT_WASH;
    w.active.weak_bg_fill = ACCENT_WASH;
    w.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = egui::Stroke::new(1.0, TEXT);

    w.open.bg_fill = CARD;
    w.open.weak_bg_fill = CARD;
    w.open.bg_stroke = egui::Stroke::new(1.0, BORDER);
    w.open.fg_stroke = egui::Stroke::new(1.0, TEXT);

    ctx.set_visuals_of(egui::Theme::Light, v.clone());
    ctx.set_visuals_of(egui::Theme::Dark, v);

    ctx.all_styles_mut(|style| {
        let radius = egui::CornerRadius::same(8);
        let w = &mut style.visuals.widgets;
        for v in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            v.corner_radius = radius;
        }

        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.visuals.selection.stroke.width = 1.0;

        // egui makes every label selectable by default, which puts a text
        // caret over all of a mini-app's UI and makes ordinary labels feel
        // like a document. An app's text is not a document — it is a
        // rendering of a guest's `ui-label` — so the pointer stays a pointer.
        style.interaction.selectable_labels = false;
    });
}

/// A tile's footprint. Fixed rather than content-sized: a grid of cards that
/// are all different widths reads as a mistake, not as a layout.
const TILE: egui::Vec2 = egui::vec2(196.0, 104.0);

struct ShellApp {
    shell: Shell,
    /// Buttons clicked during the previous egui frame, fed to the guest next.
    pending_clicks: HashSet<u32>,
    /// Live contents of the guest's text fields, by index. The host owns these
    /// buffers; the guest is told what is in them one frame later.
    field_values: HashMap<u32, String>,
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
            field_values: HashMap::new(),
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
    /// What the window is cleared to before egui draws anything.
    ///
    /// eframe's default ignores the visuals entirely and returns a hardcoded
    /// near-black, so a light theme still came up on a black window. It is the
    /// one colour the palette cannot reach from `install_theme`.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    // eframe 0.35 hands us a `&mut Ui` directly (no need to open a CentralPanel).
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // On Linux the webview lives in GTK's loop, not this one; without this
        // a page loads and then freezes. No-op on the other platforms.
        #[cfg(feature = "webview")]
        crate::webview::pump();

        // Has anything we asked for arrived? Cheap, and does nothing when
        // nothing is in flight.
        self.shell.poll();

        self.sync_window_title(ui);
        // A separator under nothing is just a line across the top of the page,
        // so the start page — which draws no chrome — gets none.
        let drew = self.chrome(ui);
        let loading = self.loading_bar(ui);
        if drew || loading {
            ui.separator();
        }

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
    /// Returns whether anything was drawn.
    ///
    /// The address bar lives here rather than on the home screen because it is
    /// the entry point for *both* modules — you must be able to type a new
    /// address without first going home, exactly like a browser.
    ///
    /// With one exception, and it is the browser's own: **the start page has no
    /// chrome.** A bar reading `mini-apps/store/store.component.wasm` above the
    /// start page was chrome describing its own furniture, and next to the
    /// store's search box it read as a second, contradictory text field. The
    /// store's field is the only place to type there, and it forwards an
    /// address through `open-app` — so navigation is still the host's to
    /// perform, it just is not the host's to *draw* on that one page.
    ///
    /// Note this keys on the store being loaded, not on `View::Home`: the
    /// built-in fallback list is host UI and keeps its address bar, or a failed
    /// store would leave the shell with nowhere to type at all.
    fn chrome(&mut self, ui: &mut egui::Ui) -> bool {
        if self.shell.is_start_page() {
            // An error still has to surface somewhere, and on this page there
            // is nowhere else.
            if self.shell.status_is_error && !self.shell.status.is_empty() {
                ui.colored_label(egui::Color32::RED, self.shell.status.as_str());
                return true;
            }
            return false;
        }

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
            // Drop any input aimed at the app we just left.
            self.pending_clicks.clear();
            self.field_values.clear();
            self.shell.go_home();
        }
        if let Some(n) = nav {
            self.navigate(n);
        }
        true
    }

    /// A browser's tab spinner: the page you are on keeps drawing while the
    /// next one is fetched, and this is the only sign that anything is
    /// happening. Returns whether it drew.
    ///
    /// It sits outside `chrome` deliberately — the start page has no chrome,
    /// and "your click did something" is exactly the feedback a page with no
    /// chrome would otherwise be missing.
    fn loading_bar(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(src) = self.shell.loading().map(str::to_string) else {
            return false;
        };
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0).color(palette::ACCENT));
            ui.label(egui::RichText::new(format!("Loading {src}")).color(palette::MUTED));
        });
        true
    }

    /// Perform a navigation requested by the chrome or a home-screen link.
    fn navigate(&mut self, nav: Nav) {
        self.pending_clicks.clear();
        self.field_values.clear();
        let (src, title) = match nav {
            Nav::Listed { src, title } => (src, title),
            // Typed text only becomes an address here, so the address bar and
            // the CLI accept exactly the same things.
            Nav::Typed(input) => registry::resolve(&input, &self.shell.apps),
        };
        // Started, not waited on — the window has frames to draw meanwhile,
        // and one of them is the spinner.
        self.shell.start(&src, &title);
    }

    /// The start page: the registry as links. The address bar that used to live
    /// here is permanent chrome now, so this is purely the bookmark list.
    fn home(&mut self, ui: &mut egui::Ui) {
        // On the way to the start page this list is not the destination, it is
        // what happens if the store fails. Showing it while the store is still
        // loading advertises a failure that has not happened.
        if self.shell.loading().is_some() {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                ui.add(egui::Spinner::new().size(28.0).color(palette::ACCENT));
            });
            return;
        }

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
        let input = FrameInput {
            clicked: std::mem::take(&mut self.pending_clicks),
            edits: self.field_values.clone(),
        };

        let cmds = match self.shell.frame(input) {
            Ok(c) => c,
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("guest error: {e}"));
                return;
            }
        };

        // A guest that asked to open another app has already been replaced by
        // it. Its widget state belongs to an instance that no longer exists.
        if self.shell.took_navigation() {
            self.pending_clicks.clear();
            self.field_values.clear();
            return;
        }

        // Translate the guest's UI commands into native egui widgets. A run of
        // consecutive tiles is taken as one unit: the grid is a property of the
        // run, not of any tile in it, and no tile knows where it sits.
        let mut i = 0;
        while i < cmds.len() {
            if matches!(cmds[i], UiCmd::Tile { .. }) {
                let end = cmds[i..]
                    .iter()
                    .position(|c| !matches!(c, UiCmd::Tile { .. }))
                    .map_or(cmds.len(), |n| i + n);
                self.tile_grid(ui, &cmds[i..end]);
                i = end;
                continue;
            }
            self.widget(ui, &cmds[i]);
            i += 1;
        }

        for l in self.shell.take_logs() {
            eprintln!("[wasm log] {l}");
        }
    }

    /// One non-tile command.
    fn widget(&mut self, ui: &mut egui::Ui, cmd: &UiCmd) {
        match cmd {
            UiCmd::Label(t) => {
                ui.label(t);
            }
            UiCmd::Button { index, text } => {
                if ui.button(text).clicked() {
                    self.pending_clicks.insert(*index);
                }
            }
            // A run of one still goes through the grid, so a lone tile is a
            // card and not a special case.
            UiCmd::Tile { .. } => self.tile_grid(ui, std::slice::from_ref(cmd)),
            UiCmd::Heading { text, level } => {
                let (size, air) = match level {
                    1 => (44.0, 22.0),
                    2 => (24.0, 12.0),
                    _ => (14.0, 6.0),
                };
                ui.add_space(air);
                ui.vertical_centered(|ui| {
                    let t = egui::RichText::new(text).size(size);
                    ui.label(match level {
                        1 => t.strong().color(palette::TEXT),
                        3 => t.color(palette::MUTED),
                        _ => t.color(palette::TEXT),
                    });
                });
                ui.add_space(air * 0.5);
            }
            UiCmd::Search {
                index,
                text,
                placeholder,
            } => {
                // Wide, centred, and capped: a search box that grows with the
                // window stops looking like one somewhere around half a screen.
                let width = ui.available_width().min(560.0);
                let mut value = text.clone();
                let mut edited = false;
                ui.vertical_centered(|ui| {
                    let resp = ui.add_sized(
                        [width, 32.0],
                        egui::TextEdit::singleline(&mut value)
                            // Same reason as the plain field below: the id is
                            // the guest's index, never egui's draw order.
                            .id(egui::Id::new(("guest-field", *index)))
                            .hint_text(placeholder.as_str())
                            .vertical_align(egui::Align::Center),
                    );
                    edited = resp.changed();
                });
                if edited {
                    self.field_values.insert(*index, value);
                }
                ui.add_space(10.0);
            }
            UiCmd::TextEdit { index, text } => {
                // The live buffer is the host's; the guest is told what is
                // in it on the next frame, the same one-frame handshake
                // clicks already use.
                //
                // The id is pinned to the guest's field index rather than
                // left to egui's draw-order counter. egui keys focus and
                // cursor position by widget id, and a guest's widget list
                // changes shape constantly — the store's own search filters
                // the rows below this very field — so a position-derived id
                // would move out from under the field being typed into and
                // drop focus on the first keystroke.
                let mut value = text.clone();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut value)
                        .id(egui::Id::new(("guest-field", *index)))
                        .desired_width(320.0),
                );
                if resp.changed() {
                    self.field_values.insert(*index, value);
                }
            }
        }
    }

    /// A run of tiles, as a centred wrapping grid of cards.
    ///
    /// Rows are chunked by hand rather than left to `horizontal_wrapped`
    /// because each row is centred individually — including a short last row,
    /// which is exactly the row a wrapping layout would leave hanging.
    fn tile_grid(&mut self, ui: &mut egui::Ui, run: &[UiCmd]) {
        let gap = ui.spacing().item_spacing.x;
        let avail = ui.available_width();
        let per_row = (((avail + gap) / (TILE.x + gap)).floor() as usize).max(1);

        for row in run.chunks(per_row) {
            let width = row.len() as f32 * TILE.x + (row.len() - 1) as f32 * gap;
            let pad = ((avail - width) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                for cmd in row {
                    let UiCmd::Tile {
                        index,
                        title,
                        subtitle,
                    } = cmd
                    else {
                        continue;
                    };
                    if tile(ui, title, subtitle) {
                        self.pending_clicks.insert(*index);
                    }
                }
            });
        }
        ui.add_space(4.0);
    }
}

/// One card. Painted rather than composed out of a button, because a button
/// with two lines of text in it is not what egui's button is for — and the
/// whole card has to be the click target, not the label inside it.
fn tile(ui: &mut egui::Ui, title: &str, subtitle: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(TILE, egui::Sense::click());
    // The whole card is the link, so it gets the cursor a link gets.
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let (fill, stroke) = if resp.hovered() {
        (palette::ACCENT_WASH, egui::Stroke::new(1.0, palette::ACCENT))
    } else {
        (palette::CARD, egui::Stroke::new(1.0, palette::BORDER))
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same(10),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );

    // The coloured initial a browser draws for a site with no favicon. There
    // are no icons to fetch here and no glyphs in egui's bundled fonts worth
    // using, so the letter is the icon.
    let inner = rect.shrink(12.0);
    let radius = 16.0;
    let centre = egui::pos2(inner.min.x + radius, inner.min.y + radius);
    painter.circle_filled(centre, radius, palette::badge(title));
    painter.text(
        centre,
        egui::Align2::CENTER_CENTER,
        title
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default(),
        egui::FontId::proportional(17.0),
        egui::Color32::WHITE,
    );

    let text_area = egui::Rect::from_min_max(
        egui::pos2(inner.min.x + radius * 2.0 + 10.0, inner.min.y),
        inner.max,
    );
    let mut card = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_area)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    // A long description must not paint over the card below it; clipping is
    // cheaper and more honest than measuring the text.
    card.set_clip_rect(text_area);
    card.label(
        egui::RichText::new(title)
            .size(15.0)
            .strong()
            .color(palette::TEXT),
    );
    card.add_space(2.0);
    card.label(
        egui::RichText::new(subtitle)
            .size(11.0)
            .color(palette::MUTED),
    );

    resp.clicked()
}
