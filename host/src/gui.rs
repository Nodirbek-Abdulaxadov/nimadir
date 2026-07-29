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
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "wasm-shell",
        native_options,
        Box::new(|_cc| Ok(Box::new(ShellApp::new(shell)) as Box<dyn eframe::App>)),
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
    /// Last title pushed to the OS window, so we only send it on change.
    last_title: String,
}

impl ShellApp {
    fn new(shell: Shell) -> Self {
        ShellApp {
            shell,
            pending_clicks: HashSet::new(),
            address: String::new(),
            last_title: String::new(),
        }
    }
}

impl eframe::App for ShellApp {
    // eframe 0.35 hands us a `&mut Ui` directly (no need to open a CentralPanel).
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sync_window_title(ui);
        self.chrome(ui);
        ui.separator();

        if self.shell.is_home() {
            self.home(ui);
        } else {
            self.page(ui);
        }

        // Immediate-mode: keep redrawing so button clicks are picked up promptly.
        ui.ctx().request_repaint();
    }
}

impl ShellApp {
    /// Mirror the current view into the OS window title, like a browser tab.
    fn sync_window_title(&mut self, ui: &egui::Ui) {
        let want = match &self.shell.view {
            View::Home => "wasm-shell".to_string(),
            View::App { title, .. } => format!("wasm-shell — {title}"),
        };
        if want != self.last_title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.last_title = want;
        }
    }

    /// The bar above the page: Home button, current title, current address.
    fn chrome(&mut self, ui: &mut egui::Ui) {
        let mut go_home = false;
        let (title, src) = match &self.shell.view {
            View::Home => ("wasm-shell".to_string(), String::new()),
            View::App { title, src, .. } => (title.clone(), src.clone()),
        };
        let at_home = self.shell.is_home();

        ui.horizontal(|ui| {
            if !at_home {
                if ui.button("Home").clicked() {
                    go_home = true;
                }
                ui.separator();
            }
            ui.strong(title.as_str());
            // Opened straight from the address bar, the title *is* the address;
            // showing it twice reads as a rendering bug.
            if !src.is_empty() && src != title {
                ui.weak(src.as_str());
            }
        });

        if go_home {
            // Drop any clicks aimed at the app we just left.
            self.pending_clicks.clear();
            self.shell.go_home();
        }
    }

    /// The start page: the registry as links, plus an address bar.
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

        ui.separator();
        ui.label("Or open a .wasm by path, URL, or the name of one listed above:");

        // Submitted with nothing typed — needs to say so, or the button looks dead.
        let mut submitted_empty = false;

        let mut addr = std::mem::take(&mut self.address);
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut addr)
                    .desired_width(520.0)
                    .hint_text("counter   |   path\\to\\app.wasm   |   https://…/app.wasm"),
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

        if submitted_empty {
            self.shell.status =
                "type a .wasm path, a URL, or the name of a listed app".to_string();
            self.shell.status_is_error = true;
        }

        if !self.shell.status.is_empty() {
            ui.add_space(8.0);
            if self.shell.status_is_error {
                ui.colored_label(egui::Color32::RED, self.shell.status.as_str());
            } else {
                ui.weak(self.shell.status.as_str());
            }
        }

        if let Some(n) = nav {
            self.pending_clicks.clear();
            let (src, title) = match n {
                Nav::Listed { src, title } => (src, title),
                // Typed text only becomes an address here, so the address bar
                // and the CLI accept exactly the same things.
                Nav::Typed(input) => registry::resolve(&input, &self.shell.apps),
            };
            self.shell.open(&src, &title);
        }
    }

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
