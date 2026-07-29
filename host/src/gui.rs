//! Native window backend (egui via eframe). Enabled with `--features gui`.
//!
//! Note: this renders with egui, NOT Makepad. Makepad was the intended primary
//! toolkit, but the mini-app boundary here is *immediate-mode* (the guest
//! re-describes its whole UI every frame), and egui is an immediate-mode
//! toolkit, so it maps 1:1 with no adapter. Makepad is retained-mode + a DSL,
//! which would need a translation layer. Both are pure-Rust, GPU-rendered, and
//! use no HTML/CSS/DOM/JS — so the "no web stack" goal holds either way. To
//! switch to Makepad later, implement the same loop against `MiniApp::frame`;
//! nothing else changes.

use std::collections::HashSet;

use anyhow::Result;
use eframe::egui;
use wasmtime::Engine;

use crate::host::MiniApp;
use crate::ui::UiCmd;

pub fn run(engine: Engine, wasm: Vec<u8>, title: String) -> Result<()> {
    let app = MiniApp::load(&engine, &wasm)?;
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        &format!("wasm-shell — {title}"),
        native_options,
        Box::new(|_cc| {
            Ok(Box::new(ShellApp {
                app,
                pending_clicks: HashSet::new(),
            }) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;
    Ok(())
}

struct ShellApp {
    app: MiniApp,
    /// Buttons clicked during the previous egui frame, fed to the guest next.
    pending_clicks: HashSet<u32>,
}

impl eframe::App for ShellApp {
    // eframe 0.35 hands us a `&mut Ui` directly (no need to open a CentralPanel).
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let clicks = std::mem::take(&mut self.pending_clicks);

        let cmds = match self.app.frame(clicks) {
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

        for l in self.app.take_logs() {
            eprintln!("[wasm log] {l}");
        }

        // Immediate-mode: keep redrawing so button clicks are picked up promptly.
        ui.ctx().request_repaint();
    }
}
