//! `runaway` — a **test fixture**, not an app. It does the three things an
//! untrusted guest must not be able to do to the shell: never return from
//! `update`, grow its memory without end, and send the host more UI than it can
//! hold. `run-tests.sh` clicks each button and checks that the host stops the
//! guest and says which limit it hit.
//!
//! The element tree (ADR-4) outlives a frame, so it has limits of its own,
//! for the whole document: buttons 6 to 8 go past each.
//!
//! It is deliberately left out of `apps.json`: nothing about it belongs on a
//! start page. Open it by path.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::hint::black_box;

use nimadir::shell::dom;
use nimadir::shell::host_api::{image_load, log, storage_set, ui_begin, ui_button, ui_label};

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

        // Button 3: keep more than an app may. Not a trap — `storage-set`
        // answers with why, and the app carries on.
        if ui_button("Overfill storage") {
            match storage_set("big", &"x".repeat(2 << 20)) {
                Ok(()) => log("runaway: stored 2 MiB"),
                Err(why) => log(&format!("runaway: {why}")),
            }
        }

        // Button 4: open a thousand groups and close none. The host draws the
        // first 32 and flattens the rest — a renderer recurses once a level,
        // and the guest does not get to choose how deep the host's stack goes.
        if ui_button("Nest deeply") {
            for _ in 0..1000 {
                ui_begin("column");
            }
            ui_label("deep");
        }

        // Button 5: 65 bytes of PNG that claim to be 100 000 pixels square —
        // 40 GB of pixels, if anyone believed it. The host reads the size from
        // the header and refuses before setting a byte aside.
        if ui_button("Load a huge image") {
            match image_load(HUGE_PNG) {
                Ok(_) => log("runaway: the huge image loaded"),
                Err(why) => log(&format!("runaway: image refused: {why}")),
            }
        }
        // Button 6: elements without end, none of them in the tree. Detached
        // nodes are still the host's to hold.
        if ui_button("Build without end") {
            loop {
                black_box(dom::create_element("div"));
            }
        }
        // Button 7: a page nested 300 levels deep. Styling and layout walk
        // the tree recursively, so its depth is the host's to bound.
        if ui_button("Nest the page deeply") {
            let mut at = dom::root();
            for _ in 0..300 {
                let n = dom::create_element("div");
                dom::append_child(at, n);
                at = n;
            }
        }
        // Button 8: a megabyte of text, again and again — far fewer nodes
        // than the cap, far more bytes.
        if ui_button("Fill the page") {
            let big = "x".repeat(1 << 20);
            loop {
                black_box(dom::create_text(&big));
            }
        }
    }
}

/// A PNG of 100000 x 100000 RGBA by its header, with an empty image stream.
const HUGE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x01, 0x86, 0xa0, 0x00, 0x01, 0x86, 0xa0, 0x08, 0x06, 0x00, 0x00, 0x00, 0xa8, 0x52, 0x0b,
    0xc8, 0x00, 0x00, 0x00, 0x08, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00,
    0x01, 0x48, 0x06, 0x89, 0xd2, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60,
    0x82,
];

export!(Runaway);
