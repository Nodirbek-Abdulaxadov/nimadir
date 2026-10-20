//! `runaway` — a **test fixture**, not an app. It does the three things an
//! untrusted guest must not be able to do to the shell: never return from
//! `update`, grow its memory without end, and send the host more UI than it can
//! hold. `run-tests.sh` clicks each button and checks that the host stops the
//! guest and says which limit it hit.
//!
//! It is deliberately left out of `apps.json`: nothing about it belongs on a
//! start page. Open it by path.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::hint::black_box;

use nimadir::shell::host_api::{log, ui_button, ui_label};

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

        // Button 1: grow linear memory until something stops it. Straight
        // through `memory.grow`, not the allocator: an allocator writes the
        // pages it hands out, and writing 512 MiB takes long enough on a busy
        // machine that the *frame* budget sometimes fired first. A test that
        // asks about one limit must not race the other. 1024 pages is 64 MiB.
        if ui_button("Eat memory") {
            loop {
                black_box(core::arch::wasm32::memory_grow(0, 1024));
            }
        }

        // Button 2: one megabyte, sent again and again. The guest holds it
        // once; the host would keep a copy per call, so it is the host's
        // memory this spends, and the guest's memory cap never sees it.
        if ui_button("Flood the shell") {
            let big = "x".repeat(1 << 20);
            loop {
                ui_label(&big);
            }
        }
    }
}

export!(Runaway);
