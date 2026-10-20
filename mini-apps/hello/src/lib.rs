//! `hello` mini-app — a second guest **component**, to prove the host runs
//! whatever component you point it at with no rebuild. Same `wit/` world as
//! `counter`, different behavior.
//!
//! It also shows time passing, which takes `request-frame`: the shell draws
//! only when something happens, so a clock has to ask for its next tick.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicI64, Ordering};

use nimadir::shell::host_api::{log, now_millis, request_frame, ui_button, ui_label};

/// When `init` ran, by the host's clock.
static STARTED: AtomicI64 = AtomicI64::new(0);

struct Hello;

impl Guest for Hello {
    fn init() {
        STARTED.store(now_millis(), Ordering::Relaxed);
        log("hello mini-app: init");
    }

    fn update() {
        ui_label("Hello from a *different* mini-app!");
        ui_label("Same host binary, swapped component — no rebuild.");
        let up = (now_millis() - STARTED.load(Ordering::Relaxed)) / 1000;
        ui_label(&format!(
            "Open for {up} s — redrawn once a second, on request."
        ));
        request_frame(1000);
        if ui_button("Say hi") {
            log("hi from the hello mini-app!");
        }
    }
}

export!(Hello);
