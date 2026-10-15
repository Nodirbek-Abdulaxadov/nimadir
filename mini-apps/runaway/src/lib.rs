//! `runaway` — a **test fixture**, not an app. It does the two things an
//! untrusted guest must not be able to do to the shell: never return from
//! `update`, and grow its memory without end. `run-tests.sh` clicks each button
//! and checks that the host stops the guest and says which limit it hit.
//!
//! It is deliberately left out of `apps.json`: nothing about it belongs on a
//! start page. Open it by path.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::hint::black_box;
use std::sync::Mutex;

use nimadir::shell::host_api::{log, ui_button, ui_label};

/// Where the eaten memory goes, so it stays allocated.
static HOARD: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

struct Runaway;

impl Guest for Runaway {
    fn init() {
        log("runaway: init");
    }

    fn update() {
        ui_label("This app misbehaves on purpose.");

        // Button 0: never return. The host's frame budget has to end this.
        if ui_button("Spin forever") {
            let mut n = 0u64;
            loop {
                n = black_box(n.wrapping_add(1));
            }
        }

        // Button 1: allocate until something stops it. Each block is written
        // to, so the pages are real and not just reserved.
        if ui_button("Eat memory") {
            let mut hoard = HOARD.lock().unwrap();
            loop {
                hoard.push(black_box(vec![1u8; 16 << 20]));
            }
        }
    }
}

export!(Runaway);
