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

use crate::i18n::{t, Lang, Text};
use crate::registry;
use crate::settings;
use crate::shell::{Shell, View, SETTINGS_PAGE};
use crate::ui::{Container, FrameInput, UiCmd};

/// What `run_android` is handed, re-exported so the Android crate need not
/// depend on winit itself just to name it.
#[cfg(target_os = "android")]
pub use winit::platform::android::activity::AndroidApp;

/// Where a home-screen interaction wants to navigate.
enum Nav {
    /// A link from the registry — already a real source.
    Listed { src: String, title: String },
    /// Raw text from the address bar — still has to be resolved.
    Typed(String),
}

pub fn run(shell: Shell) -> Result<()> {
    run_with(
        shell,
        eframe::NativeOptions {
            // The installed .desktop file's name: what a desktop matches the
            // window against for its icon and its place in the dock.
            viewport: egui::ViewportBuilder::default().with_app_id("nimadir-shell"),
            // A child webview attaches only to an X11 window (`webview::
            // check_parent`), so a build that opens web pages asks for
            // XWayland on a Wayland desktop instead of refusing every page
            // there. Only when there is an X server to ask: without one the
            // window still opens, on Wayland, and pages say why they cannot.
            #[cfg(all(feature = "webview", target_os = "linux"))]
            event_loop_builder: crate::webview::has_x11().then(|| {
                Box::new(
                    |builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>| {
                        use winit::platform::x11::EventLoopBuilderExtX11;
                        builder.with_x11();
                    },
                ) as eframe::EventLoopBuilderHook
            }),
            ..Default::default()
        },
    )
}

/// The Android front door: the same window, handed the activity it lives in.
/// eframe needs the `AndroidApp` to build its event loop; everything after
/// that is the desktop path unchanged.
#[cfg(target_os = "android")]
pub fn run_android(shell: Shell, app: AndroidApp) -> Result<()> {
    if let Some(tag) = crate::android::system_locale(&app) {
        crate::set_locale(tag);
    }
    run_with(
        shell,
        eframe::NativeOptions {
            android_app: Some(app),
            ..Default::default()
        },
    )
}

fn run_with(mut shell: Shell, native_options: eframe::NativeOptions) -> Result<()> {
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

    #[cfg(target_os = "android")]
    let android_app = native_options.android_app.clone();

    eframe::run_native(
        "nimadir",
        native_options,
        Box::new(move |cc| {
            install_theme(&cc.egui_ctx);
            #[cfg(target_os = "linux")]
            crate::portal::follow_color_scheme(cc.egui_ctx.clone());
            #[allow(unused_mut)]
            let mut app = ShellApp::new(shell);
            #[cfg(feature = "webview")]
            {
                app.web_unavailable = web_unavailable;
            }
            #[cfg(target_os = "android")]
            {
                app.safe_area = android_app.clone().map(crate::android::SafeArea::new);
                app.activity = android_app;
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

    use eframe::egui;

    /// The colours of one theme. Everything drawn by hand — cards, headings,
    /// the spinner — reads them through `of`, so it follows the theme the
    /// window is in rather than being fixed to one.
    pub struct Palette {
        pub surface: Color32,
        /// Cards and unpressed controls — the faint grey Chrome uses for chips.
        pub card: Color32,
        pub border: Color32,
        pub text: Color32,
        pub muted: Color32,
        pub accent: Color32,
        /// The wash under a hovered control.
        pub accent_wash: Color32,
        pub danger: Color32,
    }

    pub const LIGHT: Palette = Palette {
        surface: Color32::from_rgb(0xff, 0xff, 0xff),
        card: Color32::from_rgb(0xf1, 0xf3, 0xf4),
        border: Color32::from_rgb(0xda, 0xdc, 0xe0),
        text: Color32::from_rgb(0x20, 0x21, 0x24),
        muted: Color32::from_rgb(0x5f, 0x63, 0x68),
        accent: Color32::from_rgb(0x1a, 0x73, 0xe8),
        accent_wash: Color32::from_rgb(0xe8, 0xf0, 0xfe),
        danger: Color32::from_rgb(0xd9, 0x30, 0x25),
    };

    /// The same roles at night: a dark grey page rather than black, and a
    /// lighter blue, which stays readable on it.
    pub const DARK: Palette = Palette {
        surface: Color32::from_rgb(0x20, 0x21, 0x24),
        card: Color32::from_rgb(0x30, 0x31, 0x34),
        border: Color32::from_rgb(0x5f, 0x63, 0x68),
        text: Color32::from_rgb(0xe8, 0xea, 0xed),
        muted: Color32::from_rgb(0x9a, 0xa0, 0xa6),
        accent: Color32::from_rgb(0x8a, 0xb4, 0xf8),
        accent_wash: Color32::from_rgb(0x2a, 0x37, 0x4f),
        danger: Color32::from_rgb(0xf2, 0x8b, 0x82),
    };

    /// The palette of the theme `ui` is drawn in.
    pub fn of(ui: &egui::Ui) -> &'static Palette {
        if ui.visuals().dark_mode {
            &DARK
        } else {
            &LIGHT
        }
    }

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

/// How the window's scroll areas take a drag. egui turns dragging on once
/// it has seen a touch, and the frame with the first one was laid out
/// before that: on a phone the first swipe after launch did not scroll. A
/// phone is a touch screen from the start; elsewhere a mouse drag selects,
/// as in a browser, and a touch screen is found the way egui finds it.
fn scroll_source() -> egui::scroll_area::ScrollSource {
    use egui::scroll_area::{DragScroll, ScrollSource};
    ScrollSource {
        drag: if cfg!(target_os = "android") {
            DragScroll::Always
        } else {
            DragScroll::OnTouch
        },
        ..ScrollSource::default()
    }
}

/// The window's look: a browser's, in light and in dark, from the two
/// palettes, following whichever the system asks for.
///
/// When the system does not say — some Linux desktops — egui would fall back
/// to dark; the fallback here is light, the theme the shell was designed in.
/// Android never says through winit, so there the frame sets the fallback
/// from the activity's configuration (`android::night`).
fn install_theme(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_visuals_of(
        egui::Theme::Light,
        visuals(&palette::LIGHT, egui::Visuals::light()),
    );
    ctx.set_visuals_of(
        egui::Theme::Dark,
        visuals(&palette::DARK, egui::Visuals::dark()),
    );
    ctx.options_mut(|o| o.fallback_theme = egui::Theme::Light);
    ctx.set_theme(egui::ThemePreference::System);

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

        // A finger is not a pointer. On a phone every control is at least 48
        // points tall — Android's minimum touch target, since a point there is
        // a dp — where the desktop's were about 32.
        #[cfg(target_os = "android")]
        {
            style.spacing.interact_size.y = 48.0;
            style.spacing.button_padding = egui::vec2(16.0, 12.0);
        }

        // egui makes every label selectable by default, which puts a text
        // caret over all of a mini-app's UI and makes ordinary labels feel
        // like a document. An app's text is not a document — it is a
        // rendering of a guest's `ui-label` — so the pointer stays a pointer.
        style.interaction.selectable_labels = false;
    });
}

/// One theme's visuals, from its palette.
fn visuals(p: &palette::Palette, base: egui::Visuals) -> egui::Visuals {
    let mut v = base;
    v.panel_fill = p.surface;
    v.window_fill = p.surface;
    v.faint_bg_color = p.card;
    v.extreme_bg_color = p.surface;
    v.hyperlink_color = p.accent;
    v.error_fg_color = p.danger;
    v.warn_fg_color = p.danger;
    v.window_stroke = egui::Stroke::new(1.0, p.border);
    v.selection.bg_fill = p.accent_wash;
    v.selection.stroke = egui::Stroke::new(1.0, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = egui::Stroke::new(1.0, p.text);

    w.inactive.bg_fill = p.card;
    w.inactive.weak_bg_fill = p.card;
    w.inactive.bg_stroke = egui::Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = egui::Stroke::new(1.0, p.text);

    w.hovered.bg_fill = p.accent_wash;
    w.hovered.weak_bg_fill = p.accent_wash;
    w.hovered.bg_stroke = egui::Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = egui::Stroke::new(1.0, p.accent);

    // `active` is also where `strong_text_color()` comes from, so its
    // foreground has to stay legible on the page — a solid blue button with
    // white text here would turn every bold label white on white.
    w.active.bg_fill = p.accent_wash;
    w.active.weak_bg_fill = p.accent_wash;
    w.active.bg_stroke = egui::Stroke::new(1.0, p.accent);
    w.active.fg_stroke = egui::Stroke::new(1.0, p.text);

    w.open.bg_fill = p.card;
    w.open.weak_bg_fill = p.card;
    w.open.bg_stroke = egui::Stroke::new(1.0, p.border);
    w.open.fg_stroke = egui::Stroke::new(1.0, p.text);
    v
}

/// egui's own fonts, plus Noto Sans after them as a fallback.
///
/// None of egui's bundled fonts has U+02BB — the ʻ in Uzbek's oʻ and gʻ — so
/// Uzbek Latin drew as boxes. Checked against the fonts' own character maps:
/// Cyrillic, Uzbek's included, they already cover; arrows the proportional
/// font lacks and Noto does not add — the chrome says "Back", not "←". Last in
/// each family, so egui's look is unchanged and Noto only draws what nothing
/// before it can. Licence: `host/assets/fonts/NotoSans-OFL.txt`.
///
/// An app's element tree (ADR-4) is drawn in Noto Sans throughout, regular
/// and bold (`gui_dom::NOTO`, `NOTO_BOLD`), so its weights are one face;
/// egui's own fonts follow as fallbacks there, for emoji and the like.
fn install_fonts(ctx: &egui::Context) {
    use crate::gui_dom::{NOTO, NOTO_BOLD};
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            NOTO,
            &include_bytes!("../assets/fonts/NotoSans-Regular.ttf")[..],
        ),
        (
            NOTO_BOLD,
            &include_bytes!("../assets/fonts/NotoSans-Bold.ttf")[..],
        ),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let fallbacks = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(NOTO.to_owned());
    }
    for (family, faces) in [
        (NOTO, vec![NOTO.to_owned()]),
        (NOTO_BOLD, vec![NOTO_BOLD.to_owned(), NOTO.to_owned()]),
    ] {
        let mut list = faces;
        list.extend(fallbacks.iter().cloned());
        fonts
            .families
            .insert(egui::FontFamily::Name(family.into()), list);
    }
    ctx.set_fonts(fonts);
}

/// A tile's footprint. Fixed rather than content-sized: a grid of cards that
/// are all different widths reads as a mistake, not as a layout.
const TILE: egui::Vec2 = egui::vec2(196.0, 104.0);

struct ShellApp {
    shell: Shell,
    /// Buttons clicked during the previous egui frame, fed to the guest next.
    pending_clicks: HashSet<u32>,
    /// Text fields submitted during the previous frame, fed to the guest next.
    pending_submits: HashSet<u32>,
    /// What the window keeps of an app's element tree between frames.
    dom: crate::gui_dom::DomView,
    /// Guest images uploaded to the GPU, by `Picture::id`. Kept while drawn;
    /// one not drawn in a frame is dropped, with the app it belonged to.
    textures: HashMap<u64, egui::TextureHandle>,
    /// The images drawn in the current frame.
    textures_used: HashSet<u64>,
    /// Live contents of the guest's text fields, by index. The host owns these
    /// buffers; the guest is told what is in them one frame later.
    field_values: HashMap<u32, String>,
    /// Address bar contents.
    address: String,
    /// The address the bar was last synced to, so navigation can refresh it
    /// without overwriting what the user is in the middle of typing.
    synced_src: String,
    /// The app the page's scroll position belongs to; another app starts at
    /// the top.
    scrolled_for: String,
    /// Whether the page was taller than its area at the end of the last
    /// frame: what egui's scroll area reads, a frame late, to take a drag.
    page_overflows: bool,
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
    /// Where the system's bars are not. Set once the window exists, from the
    /// activity eframe was handed.
    #[cfg(target_os = "android")]
    safe_area: Option<crate::android::SafeArea>,
    /// The activity itself: the system's theme, and leaving on Back.
    #[cfg(target_os = "android")]
    activity: Option<AndroidApp>,
    /// Whether the system was in its dark theme when last looked at; `None`
    /// until the first frame has set the theme and the bars to match.
    #[cfg(target_os = "android")]
    night: Option<bool>,
    /// Whether the on-screen keyboard was up last frame.
    #[cfg(target_os = "android")]
    keyboard_up: bool,
    /// The keyboard has just come up: the focused field scrolls into what is
    /// left of the page. Only ever set on Android.
    reveal_focus: bool,
    /// Something done in this frame shows only in the next — a click or an
    /// edit the guest has not been told about yet — so ask for one.
    redraw: bool,
    /// What the settings page shows, read from disk once per visit rather
    /// than on every frame it is drawn.
    settings_seen: Option<(Vec<settings::Kept>, u64)>,
    /// The language the window speaks, from the user's locale.
    lang: Lang,
    /// A check for a newer release the user asked for: the answer on its way,
    /// then the answer, worded.
    update_rx: Option<std::sync::mpsc::Receiver<Result<crate::update::Verdict, String>>>,
    update_said: Option<String>,
}

impl ShellApp {
    fn new(shell: Shell) -> Self {
        ShellApp {
            shell,
            pending_clicks: HashSet::new(),
            pending_submits: HashSet::new(),
            dom: crate::gui_dom::DomView::default(),
            textures: HashMap::new(),
            textures_used: HashSet::new(),
            field_values: HashMap::new(),
            address: String::new(),
            synced_src: String::new(),
            scrolled_for: String::new(),
            page_overflows: false,
            last_title: String::new(),
            #[cfg(feature = "webview")]
            web: None,
            #[cfg(feature = "webview")]
            web_unavailable: None,
            #[cfg(feature = "webview")]
            web_error: None,
            #[cfg(target_os = "android")]
            safe_area: None,
            #[cfg(target_os = "android")]
            activity: None,
            #[cfg(target_os = "android")]
            night: None,
            #[cfg(target_os = "android")]
            keyboard_up: false,
            reveal_focus: false,
            redraw: false,
            settings_seen: None,
            lang: Lang::current(),
            update_rx: None,
            update_said: None,
        }
    }

    /// The settings page: host UI, at `SETTINGS_PAGE`. What each app keeps,
    /// with a way to clear it; what the caches hold; and where both live.
    fn settings(&mut self, ui: &mut egui::Ui) {
        let (kept, cache) = self
            .settings_seen
            .get_or_insert_with(|| (settings::kept(), settings::cache_bytes()));

        let mut clear: Option<String> = None;
        let mut clear_cache = false;
        let mut check_update = false;
        // An answer that has come in.
        if let Some(Ok(verdict)) = self.update_rx.as_ref().map(|rx| rx.try_recv()) {
            self.update_rx = None;
            self.update_said = Some(match verdict {
                Ok(crate::update::Verdict::Current) => t(self.lang, Text::Newest).to_string(),
                Ok(crate::update::Verdict::Newer { version, page }) => t(self.lang, Text::NewerOut)
                    .replace("{version}", &version)
                    .replace("{page}", &page),
                Err(why) => why,
            });
        }
        egui::ScrollArea::vertical()
            .scroll_source(scroll_source())
            .show(ui, |ui| {
                ui.heading(t(self.lang, Text::Settings));
                ui.weak(format!(
                    "nimadir {} — WIT {}",
                    env!("CARGO_PKG_VERSION"),
                    crate::wit_package()
                ));
                ui.add_space(12.0);

                ui.strong(t(self.lang, Text::WhatAppsKeep));
                if kept.is_empty() {
                    ui.weak(t(self.lang, Text::NothingKept));
                }
                for k in kept.iter() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(&k.identity);
                        ui.weak(settings::human(k.bytes));
                        if ui.button(t(self.lang, Text::ClearData)).clicked() {
                            clear = Some(k.identity.clone());
                        }
                    });
                }
                ui.add_space(12.0);

                ui.strong(t(self.lang, Text::Caches));
                ui.horizontal_wrapped(|ui| {
                    ui.label(settings::human(*cache));
                    if ui.button(t(self.lang, Text::ClearCache)).clicked() {
                        clear_cache = true;
                    }
                });
                ui.weak(t(self.lang, Text::CachesRebuild));
                ui.add_space(12.0);

                ui.strong(t(self.lang, Text::Updates));
                ui.horizontal_wrapped(|ui| {
                    if self.update_rx.is_some() {
                        ui.spinner();
                        ui.weak(t(self.lang, Text::Checking));
                    } else if ui.button(t(self.lang, Text::CheckForUpdates)).clicked() {
                        check_update = true;
                    }
                });
                if let Some(said) = &self.update_said {
                    ui.label(said);
                }
                ui.add_space(12.0);

                for (what, dir) in [
                    (t(self.lang, Text::DataDir), crate::paths::data_dir()),
                    (t(self.lang, Text::CacheDir), crate::paths::cache_dir()),
                ] {
                    if let Some(dir) = dir {
                        ui.weak(format!("{what}: {}", dir.display()));
                    }
                }
            });

        if check_update {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::update::check());
            });
            self.update_rx = Some(rx);
            self.update_said = None;
        }
        if self.update_rx.is_some() {
            // Polled, not spun on: it is a network request.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }

        let done = match (clear, clear_cache) {
            (Some(identity), _) => Some(settings::clear(&identity)),
            (None, true) => Some(settings::clear_cache()),
            (None, false) => None,
        };
        if let Some(result) = done {
            self.shell.status_is_error = result.is_err();
            self.shell.status = result.err().unwrap_or_default();
            self.settings_seen = None;
            self.redraw = true;
        }
    }

    /// Back, from the chrome's button or the system's key.
    ///
    /// With nowhere left to go back to, a phone leaves the app, as every
    /// Android app does on Back from its first screen. A desktop window does
    /// not close itself because a mouse button was pressed.
    fn go_back(&mut self, ctx: &egui::Context) {
        // Input aimed at the page being left goes nowhere.
        self.pending_clicks.clear();
        self.pending_submits.clear();
        self.field_values.clear();
        let nowhere = !self.shell.back();
        // To the background, not closed: see `android::to_background`.
        #[cfg(target_os = "android")]
        if nowhere
            && !self
                .activity
                .as_ref()
                .is_some_and(crate::android::to_background)
        {
            crate::diag::line("Back: could not move the task to the background");
        }
        let _ = (nowhere, ctx);
    }

    /// The part of the window nothing of the system's covers — everything,
    /// chrome included, is laid out inside it.
    ///
    /// On a phone the window runs under the status bar and the navigation bar,
    /// and a tap on either goes to the system: laid out over the whole window,
    /// the Home button sat under the clock where it could not be pressed. A
    /// small gutter keeps fields off the screen's edges too. On the desktop
    /// there is nothing to avoid, and it is the window less a margin.
    fn safe_rect(&mut self, ui: &egui::Ui) -> egui::Rect {
        let full = ui.max_rect();
        #[cfg(target_os = "android")]
        if let Some(area) = self.safe_area.as_mut() {
            /// Side margin on a phone, in points, where the bars leave none.
            const GUTTER: f32 = 12.0;
            let typing = ui.ctx().egui_wants_keyboard_input();
            let px = area.insets((full.width(), full.height()), typing);
            // The keyboard slides in and out without a redraw to notice it
            // by, so look again shortly for as long as it may be moving.
            if typing || px.keyboard {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(200));
            }
            // Just come up: the field it is for may now be under it.
            self.reveal_focus = px.keyboard && !self.keyboard_up;
            self.keyboard_up = px.keyboard;
            let ppp = ui.ctx().pixels_per_point();
            return egui::Rect::from_min_max(
                full.min + egui::vec2((px.left / ppp).max(GUTTER), px.top / ppp),
                full.max - egui::vec2((px.right / ppp).max(GUTTER), px.bottom / ppp),
            );
        }
        // A desktop window's edges are not a finger's problem, but text
        // touching them is hard to read: egui's own panels leave 8 points.
        full.shrink(8.0)
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

        // winit reports no system theme on Android, so egui would always use
        // the fallback; the activity's configuration says which it should be,
        // and the system bars' icons follow.
        #[cfg(target_os = "android")]
        if let Some(app) = &self.activity {
            let night = crate::android::night(app);
            if self.night != Some(night) {
                self.night = Some(night);
                let theme = if night {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                };
                ui.ctx().options_mut(|o| o.fallback_theme = theme);
                crate::android::set_bar_icons(app, !night);
                ui.ctx().request_repaint();
            }
        }

        // The system Back key on a phone, a mouse's back button on a desktop:
        // both go where the chrome's Back button goes.
        if ui.ctx().input(|i| i.key_pressed(egui::Key::BrowserBack)) {
            self.go_back(ui.ctx());
        }

        self.sync_window_title(ui);

        let safe = self.safe_rect(ui);
        ui.scope_builder(egui::UiBuilder::new().max_rect(safe), |ui| {
            // A separator under nothing is just a line across the top of the
            // page, so the start page — which draws no chrome — gets none.
            let drew = self.chrome(ui);
            let loading = self.loading_bar(ui);
            if drew || loading {
                ui.separator();
            }

            // Read the discriminant first: the arms take `&mut self`, so the
            // match cannot hold a borrow of `self.shell.view` across them.
            match Body::of(&self.shell) {
                Body::Home => {
                    self.close_web();
                    self.home(ui);
                }
                Body::App => {
                    self.close_web();
                    // A page longer than the window scrolls, as any page does
                    // — on a phone there is no making the window taller — and
                    // a guest's own "scroll" groups scroll inside it. Back at
                    // the top for each app opened: the one before it was
                    // destroyed, and where it was scrolled to with it.
                    let here = self.shell.location().map(|(_, src)| src.to_string());
                    let here = here.unwrap_or_default();
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt("page")
                        .scroll_source(scroll_source())
                        .auto_shrink([false, false]);
                    if here != self.scrolled_for {
                        area = area.vertical_scroll_offset(0.0);
                        self.scrolled_for = here;
                    }
                    let out = area.show(ui, |ui| self.page(ui));
                    // egui lets a drag scroll the page only once a frame has
                    // ended with it too tall, and this window draws only
                    // when something happens: without one more frame, the
                    // first swipe on a page just opened did not scroll it.
                    let overflows = out.content_size.y > out.inner_rect.height();
                    if overflows != self.page_overflows {
                        self.page_overflows = overflows;
                        ui.ctx().request_repaint();
                    }
                }
                Body::WebPage => self.web_page(ui, frame),
                Body::Settings => {
                    self.close_web();
                    self.settings(ui);
                }
            }
            if !matches!(Body::of(&self.shell), Body::Settings) {
                // Read afresh on the next visit: an app may have kept more.
                self.settings_seen = None;
            }
        });

        // Draw again only when the next frame will differ. Input already wakes
        // egui up; what it cannot know is that a click or an edit made in this
        // frame reaches the guest only in the next one, that a load is in
        // flight, or that the guest asked for a frame of its own. Redrawing
        // unconditionally kept a core busy on an idle page — on a phone, even
        // in the background. A webview's GTK loop is pumped from here, so a
        // page keeps the frames coming.
        let wanted = self.shell.take_frame_request();
        // Hidden — minimized, covered, or an app sent to the background — a
        // guest's own clock waits: nobody would see the frame it asked for, and
        // on a phone it is battery. Showing the window wakes egui by itself,
        // and the guest asks again from that frame.
        let hidden = ui.ctx().input(|i| {
            let v = i.viewport();
            v.minimized == Some(true) || v.occluded == Some(true)
        });
        let wanted = wanted.filter(|_| !hidden);
        // A registry on its way is polled rather than spun on: it can take
        // seconds, and nothing on screen moves while it does.
        let registry = self.shell.refreshing().then_some(250);
        let busy = self.shell.loading().is_some()
            || (cfg!(feature = "webview") && self.shell.is_web_page());
        if std::mem::take(&mut self.redraw) || busy {
            ui.ctx().request_repaint();
        } else if let Some(ms) = [wanted, registry].into_iter().flatten().min() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(ms.into()));
        }
    }
}

/// Room above and below a one-line field's text, so the field is as tall as
/// a button: on a phone, a finger's target; on a desktop, what egui gives it
/// anyway. (A `TextEdit`'s `min_size` sets only its width.)
fn field_margin(ui: &egui::Ui) -> egui::Margin {
    let row = ui.text_style_height(&egui::TextStyle::Body);
    let pad = ((ui.spacing().interact_size.y - row) / 2.0).max(2.0);
    egui::Margin::symmetric(4, pad.round() as i8)
}

/// A choice's options, as selectable labels; the one tapped, if any.
fn choices(ui: &mut egui::Ui, options: &[String], selected: u32) -> Option<usize> {
    let mut picked = None;
    for (i, option) in options.iter().enumerate() {
        if ui.selectable_label(i as u32 == selected, option).clicked() {
            picked = Some(i);
        }
    }
    picked
}

/// The narrowest address bar worth drawing beside the chrome's buttons, in
/// points; with less, it goes on a row of its own.
const MIN_ADDRESS: f32 = 200.0;

/// What the Open button takes on a row, its spacing included.
fn open_width(ui: &egui::Ui, lang: Lang) -> f32 {
    let open = egui::WidgetText::from(t(lang, Text::Open)).into_galley(
        ui,
        None,
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    open.size().x + ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x
}

/// The address bar and its Open button, filling what is left of the row up to
/// 520 pt. True when the user asked to go: Open, or Enter in the bar.
fn address_bar(ui: &mut egui::Ui, lang: Lang, addr: &mut String) -> bool {
    let width = (ui.available_width() - open_width(ui, lang)).clamp(80.0, 520.0);
    let resp = ui.add(
        egui::TextEdit::singleline(addr)
            .desired_width(width)
            .margin(field_margin(ui))
            .hint_text("counter   |   path/to/app.wasm   |   https://example.com"),
    );
    let entered = resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(egui::Key::Enter));
    ui.button(t(lang, Text::Open)).clicked() || entered
}

/// Which renderer the area below the chrome needs this frame.
enum Body {
    Home,
    App,
    WebPage,
    Settings,
}

impl Body {
    fn of(shell: &Shell) -> Self {
        match &shell.view {
            View::Home => Body::Home,
            View::App { .. } => Body::App,
            View::WebPage { .. } => Body::WebPage,
            View::Settings => Body::Settings,
        }
    }
}

impl ShellApp {
    /// Mirror the current view into the OS window title, like a browser tab.
    fn sync_window_title(&mut self, ui: &egui::Ui) {
        // "Page — browser", the way a browser's tab reads; the start page is
        // just the shell's name.
        let want = match &self.shell.view {
            View::Home => "nimadir".to_string(),
            View::App { .. } if self.shell.is_start_page() => "nimadir".to_string(),
            View::App { title, .. } | View::WebPage { title, .. } => format!("{title} — nimadir"),
            View::Settings => format!("{} — nimadir", t(self.lang, Text::Settings)),
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
        // Whether an app may reach an origin (ADR-3). Above everything, and on
        // every page — the start page included, which has no other chrome —
        // since a request waits on it.
        let asked = self.network_question(ui);

        if self.shell.is_start_page() {
            if asked {
                return true;
            }
            // An error still has to surface somewhere, and on this page there
            // is nowhere else.
            if self.shell.status_is_error && !self.shell.status.is_empty() {
                ui.colored_label(egui::Color32::RED, self.shell.status.as_str());
                return true;
            }
            return false;
        }

        let (title, src) = match &self.shell.view {
            View::Home => ("nimadir".to_string(), String::new()),
            // Titled as its address, so no title row: the page has a heading.
            View::Settings => (SETTINGS_PAGE.to_string(), SETTINGS_PAGE.to_string()),
            View::App { title, src, .. } => (title.clone(), src.clone()),
            View::WebPage { title, url } => (title.clone(), url.clone()),
        };
        let at_home = self.shell.is_home();
        let lang = self.lang;
        let can_back = self.shell.can_go_back();
        let can_forward = self.shell.can_go_forward();

        // Follow navigation, like a browser's bar does: whatever you arrived at
        // is what it shows. Only on an actual change, so typing is never eaten.
        // A listed app shows as its name — what you would type to open it —
        // not where the registry keeps it, which on a phone is a path deep in
        // the app's private storage.
        let shown = self
            .shell
            .apps
            .iter()
            .find(|e| e.src == src)
            .map_or_else(|| src.clone(), |e| e.name.clone());
        if shown != self.synced_src {
            self.address = shown.clone();
            self.synced_src = shown.clone();
        }

        let mut go_back = false;
        let mut go_forward = false;
        let mut go_home = false;
        let mut reload = false;
        let mut nav: Option<Nav> = None;
        // Submitted with nothing typed — needs to say so, or the button looks dead.
        let mut submitted_empty = false;

        let mut addr = std::mem::take(&mut self.address);
        let mut submitted = false;
        let mut wrapped = false;
        ui.horizontal(|ui| {
            // A word, not an arrow: egui's bundled fonts have no arrow glyphs.
            if can_back && ui.button(t(lang, Text::Back)).clicked() {
                go_back = true;
            }
            // Only once Back has been pressed, as in a browser — and on a
            // phone, which has a key for Back and none for Forward.
            if can_forward && ui.button(t(lang, Text::Forward)).clicked() {
                go_forward = true;
            }
            if !at_home {
                if ui.button(t(lang, Text::Home)).clicked() {
                    go_home = true;
                }
                if ui.button(t(lang, Text::Reload)).clicked() {
                    reload = true;
                }
            }
            // On a phone the buttons leave too little for an address anyone
            // could read — squeezed in, it pushed Open off the screen — so
            // the bar takes a row of its own there.
            let separator = ui.spacing().item_spacing.x * 2.0 + 1.0;
            if ui.available_width() - separator < MIN_ADDRESS + open_width(ui, lang) {
                wrapped = true;
                return;
            }
            if !at_home {
                ui.separator();
            }
            submitted = address_bar(ui, lang, &mut addr);
        });
        if wrapped {
            ui.horizontal(|ui| submitted = address_bar(ui, lang, &mut addr));
        }
        if submitted {
            if addr.trim().is_empty() {
                submitted_empty = true;
            } else {
                nav = Some(Nav::Typed(addr.clone()));
            }
        }
        self.address = addr;

        // Opened straight from the address bar, the title *is* the address, and
        // the bar above is already showing it.
        // Cut short with an ellipsis: drawn at full length, a long title made
        // the page wider than a phone, and everything under it ran off the edge.
        if !at_home && title != shown {
            ui.add(egui::Label::new(egui::RichText::new(title.as_str()).strong()).truncate());
        }

        if submitted_empty {
            self.shell.status = t(self.lang, Text::TypeSomething).to_string();
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

        // An app asking to go somewhere the registry does not list. In the
        // chrome, which the app cannot draw over or click for the user.
        let mut answer = None;
        if let Some(ask) = self.shell.asking() {
            // Escaped: a bidi override in an address would make the question
            // show somewhere other than where the answer goes.
            let question = t(self.lang, Text::WantsToOpen)
                .replace("{app}", &crate::text::visible(&ask.from))
                .replace("{address}", &crate::text::visible(&ask.address));
            let lang = self.lang;
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(question).strong());
                if ui.button(t(lang, Text::Open)).clicked() {
                    answer = Some(true);
                }
                if ui.button(t(lang, Text::Stay)).clicked() {
                    answer = Some(false);
                }
            });
        }
        if let Some(open) = answer {
            self.shell.answer(open);
        }

        if go_back {
            self.go_back(ui.ctx());
        }
        if go_forward {
            // Input aimed at the page being left goes nowhere.
            self.pending_clicks.clear();
            self.pending_submits.clear();
            self.field_values.clear();
            self.shell.forward();
        }
        if reload {
            self.pending_clicks.clear();
            self.pending_submits.clear();
            self.field_values.clear();
            self.shell.reload();
        }
        if go_home {
            // Drop any input aimed at the app we just left.
            self.pending_clicks.clear();
            self.pending_submits.clear();
            self.field_values.clear();
            self.shell.go_home();
        }
        if let Some(n) = nav {
            self.navigate(n);
        }
        true
    }

    /// The question an app's first request to an origin raises, and the
    /// buttons that answer it. Returns whether it drew.
    fn network_question(&mut self, ui: &mut egui::Ui) -> bool {
        let Some((app, origin)) = self.shell.network_question() else {
            return false;
        };
        let lang = self.lang;
        let mut answer = None;
        ui.horizontal_wrapped(|ui| {
            let question = t(lang, Text::WantsToConnect)
                .replace("{app}", &crate::text::visible(&app))
                .replace("{origin}", &crate::text::visible(&origin));
            ui.label(egui::RichText::new(question).strong());
            if ui.button(t(lang, Text::Allow)).clicked() {
                answer = Some(true);
            }
            if ui.button(t(lang, Text::DontAllow)).clicked() {
                answer = Some(false);
            }
        });
        if let Some(allow) = answer {
            self.shell.answer_network(allow);
            self.redraw = true;
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
            let p = palette::of(ui);
            ui.add(egui::Spinner::new().size(14.0).color(p.accent));
            let loading = t(self.lang, Text::Loading).replace("{address}", &src);
            ui.label(egui::RichText::new(loading).color(p.muted));
        });
        true
    }

    /// Perform a navigation requested by the chrome or a home-screen link.
    fn navigate(&mut self, nav: Nav) {
        self.pending_clicks.clear();
        self.pending_submits.clear();
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
                ui.add(
                    egui::Spinner::new()
                        .size(28.0)
                        .color(palette::of(ui).accent),
                );
            });
            return;
        }

        // Where this frame wants to navigate, if anywhere.
        let mut nav: Option<Nav> = None;

        ui.add_space(4.0);
        ui.label(t(self.lang, Text::MiniApps));
        ui.add_space(8.0);

        // Cloned so the loop holds no borrow of `self.shell` while we record a
        // click that will mutate it right after.
        let apps = self.shell.apps.clone();
        if apps.is_empty() {
            ui.weak(t(self.lang, Text::NoMiniApps));
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
        ui.strong(t(self.lang, Text::WebPage));
        ui.add_space(4.0);
        ui.label(url.as_str());
        ui.add_space(10.0);
        ui.weak(t(self.lang, Text::WebPageRouted));
        // A desktop can be rebuilt with a webview; a phone cannot have one yet.
        #[cfg(not(feature = "webview"))]
        {
            ui.add_space(6.0);
            let why = if cfg!(target_os = "android") {
                Text::WebPageNotHere
            } else {
                Text::WebPageRebuild
            };
            ui.weak(t(self.lang, why));
        }
    }

    /// Put the webview over the page area, creating it on first use.
    ///
    /// The bounds are recomputed every frame from the space egui has left, so
    /// the page follows a window resize without any resize plumbing of its own.
    #[cfg(feature = "webview")]
    fn show_web(&mut self, frame: &eframe::Frame, url: &str, ui: &egui::Ui) -> anyhow::Result<()> {
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
        self.textures_used.clear();
        let fields_before = self.field_values.clone();
        let input = FrameInput {
            clicked: std::mem::take(&mut self.pending_clicks),
            edits: self.field_values.clone(),
            submitted: std::mem::take(&mut self.pending_submits),
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
            self.pending_submits.clear();
            self.field_values.clear();
            self.dom.clear();
            return;
        }

        // The app's element tree (ADR-4), when it has built one; its
        // immediate-mode widgets follow it.
        if self.shell.document().is_some_and(|d| !d.is_empty())
            && crate::gui_dom::show(ui, &mut self.shell, &mut self.dom, self.reveal_focus)
        {
            self.redraw = true;
        }
        self.draw_run(ui, &cmds);
        let used = &self.textures_used;
        self.textures.retain(|id, _| used.contains(id));

        for l in self.shell.take_logs() {
            eprintln!("[wasm log] {l}");
        }

        // The guest hears about these next frame, so there has to be one.
        if !self.pending_clicks.is_empty()
            || !self.pending_submits.is_empty()
            || self.field_values != fields_before
        {
            self.redraw = true;
        }
    }

    /// Draw commands until the `End` that closes the group they are in, or
    /// until they run out; returns how many it used, that `End` included.
    ///
    /// Recursive, one level per group — which is why the host caps nesting
    /// (`MAX_NESTING`): the depth of this stack is not the guest's to choose.
    /// A run of consecutive tiles is taken as one unit: the grid is a
    /// property of the run, not of any tile in it, and no tile knows where it
    /// sits.
    fn draw_run(&mut self, ui: &mut egui::Ui, cmds: &[UiCmd]) -> usize {
        let mut i = 0;
        while i < cmds.len() {
            match &cmds[i] {
                UiCmd::End => return i + 1,
                UiCmd::Begin { index, kind } => {
                    let rest = &cmds[i + 1..];
                    let used = match kind {
                        Container::Row => ui.horizontal_wrapped(|ui| self.draw_run(ui, rest)).inner,
                        Container::Column => ui.vertical(|ui| self.draw_run(ui, rest)).inner,
                        Container::Section => {
                            egui::Frame::group(ui.style())
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    self.draw_run(ui, rest)
                                })
                                .inner
                        }
                        // Keyed by the guest's group number, like a field by
                        // its index, so the scroll position stays with the
                        // group when what is around it changes.
                        Container::Scroll => {
                            egui::ScrollArea::vertical()
                                .id_salt(("guest-scroll", *index))
                                .scroll_source(scroll_source())
                                .auto_shrink([false, true])
                                .show(ui, |ui| self.draw_run(ui, rest))
                                .inner
                        }
                    };
                    i += 1 + used;
                }
                UiCmd::Tile { .. } => {
                    let end = cmds[i..]
                        .iter()
                        .position(|c| !matches!(c, UiCmd::Tile { .. }))
                        .map_or(cmds.len(), |n| i + n);
                    self.tile_grid(ui, &cmds[i..end]);
                    i = end;
                }
                other => {
                    self.widget(ui, other);
                    i += 1;
                }
            }
        }
        i
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
                        1 => t.strong().color(palette::of(ui).text),
                        3 => t.color(palette::of(ui).muted),
                        _ => t.color(palette::of(ui).text),
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
                let reveal = self.reveal_focus;
                let mut value = text.clone();
                let mut edited = false;
                let mut submitted = false;
                ui.vertical_centered(|ui| {
                    let resp = ui.add_sized(
                        [width, ui.spacing().interact_size.y.max(32.0)],
                        egui::TextEdit::singleline(&mut value)
                            // Same reason as the plain field below: the id is
                            // the guest's index, never egui's draw order.
                            .id(egui::Id::new(("guest-field", *index)))
                            .hint_text(placeholder.as_str())
                            .vertical_align(egui::Align::Center),
                    );
                    edited = resp.changed();
                    submitted = submitted_by(ui, &resp);
                    if reveal && resp.has_focus() {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                });
                if edited {
                    self.field_values.insert(*index, value);
                }
                if submitted {
                    self.pending_submits.insert(*index);
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
                        .desired_width(ui.available_width().min(320.0))
                        .margin(field_margin(ui)),
                );
                if resp.changed() {
                    self.field_values.insert(*index, value);
                }
                if submitted_by(ui, &resp) {
                    self.pending_submits.insert(*index);
                }
                if self.reveal_focus && resp.has_focus() {
                    resp.scroll_to_me(Some(egui::Align::Center));
                }
            }
            UiCmd::Checkbox {
                index,
                text,
                checked,
            } => {
                let mut on = *checked;
                let resp = ui.scope(|ui| {
                    // The theme's rounding turned the small box into a circle,
                    // which is a radio button's shape, not a checkbox's.
                    let w = &mut ui.visuals_mut().widgets;
                    for v in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                        v.corner_radius = egui::CornerRadius::same(3);
                    }
                    ui.checkbox(&mut on, text)
                });
                if resp.inner.clicked() {
                    self.pending_clicks.insert(*index);
                }
            }
            // A row of options rather than a dropdown: a popup is fiddly under
            // a finger, and every option stays visible.
            UiCmd::Choice {
                index,
                options,
                selected,
            } => {
                // Each option drawn whole: squeezed in beside a field, they
                // broke a letter to a line. Inside a row they join it and
                // wrap onto its next line as needed — a row of their own
                // there would get only what is left of this one's.
                let picked = if ui.layout().main_dir().is_horizontal() {
                    let wrap = ui.style().wrap_mode;
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    let picked = choices(ui, options, *selected);
                    ui.style_mut().wrap_mode = wrap;
                    picked
                } else {
                    ui.horizontal_wrapped(|ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        choices(ui, options, *selected)
                    })
                    .inner
                };
                if let Some(i) = picked {
                    self.field_values.insert(*index, i.to_string());
                }
            }
            UiCmd::Separator => {
                ui.separator();
            }
            UiCmd::Progress(fraction) => {
                ui.add(egui::ProgressBar::new(*fraction).show_percentage());
            }
            // Uploaded once and drawn by handle; at its own size where there
            // is room, scaled down where there is not, never up.
            UiCmd::Image(picture) => {
                let texture = self.textures.entry(picture.id).or_insert_with(|| {
                    ui.ctx().load_texture(
                        format!("guest-image-{}", picture.id),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [picture.width as usize, picture.height as usize],
                            &picture.rgba,
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                });
                self.textures_used.insert(picture.id);
                let (w, h) = (picture.width as f32, picture.height as f32);
                let scale = (ui.available_width() / w).min(1.0);
                ui.image((texture.id(), egui::vec2(w * scale, h * scale)));
            }
            // Groups are `draw_run`'s; they never reach here.
            UiCmd::Begin { .. } | UiCmd::End => {}
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
    // Painted by hand, so nothing tells a screen reader what it is unless this
    // does: a button, named by what is written on it.
    resp.widget_info(|| {
        let name = if subtitle.is_empty() {
            title.to_string()
        } else {
            format!("{title}. {subtitle}")
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
    });
    // The whole card is the link, so it gets the cursor a link gets.
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let p = palette::of(ui);
    let (fill, stroke) = if resp.hovered() {
        (p.accent_wash, egui::Stroke::new(1.0, p.accent))
    } else {
        (p.card, egui::Stroke::new(1.0, p.border))
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
    card.label(egui::RichText::new(title).size(15.0).strong().color(p.text));
    card.add_space(2.0);
    card.label(egui::RichText::new(subtitle).size(11.0).color(p.muted));

    resp.clicked()
}

/// Whether a text field was submitted this frame: Enter on a keyboard, or a
/// phone keyboard's action key, which arrives as Enter — focus lost *by* it,
/// not by a tap elsewhere.
fn submitted_by(ui: &egui::Ui, resp: &egui::Response) -> bool {
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}
