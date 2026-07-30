//! `hello` mini-app — a second guest **component**, to prove the host runs
//! whatever component you point it at with no rebuild. Same `wit/` world as
//! `counter`, different behavior.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use nimadir::shell::host_api::{log, ui_button, ui_label};

struct Hello;

impl Guest for Hello {
    fn init() {
        log("hello mini-app: init");
    }

    fn update() {
        ui_label("Hello from a *different* mini-app!");
        ui_label("Same host binary, swapped component — no rebuild.");
        if ui_button("Say hi") {
            log("hi from the hello mini-app!");
        }
    }
}

export!(Hello);
