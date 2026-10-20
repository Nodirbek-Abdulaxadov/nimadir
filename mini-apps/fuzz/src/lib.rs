//! `fuzz` — a **test fixture**, not an app: every function in `host-api`,
//! called each frame with inputs a careless or hostile guest might send —
//! empty and long strings, out-of-range numbers, NaN, garbage for an image,
//! nonsense for a URL, a request number nobody issued. Deterministic: the
//! same seed every run, so a failure can be replayed.
//!
//! The host must answer all of it without panicking; the suite runs a few
//! hundred frames and checks the app is still drawing at the end.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use nimadir::shell::host_api::*;

static SEED: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
static FRAME: AtomicU32 = AtomicU32::new(0);

/// xorshift64*: small, and enough to wander the input space.
fn next() -> u64 {
    let mut x = SEED.load(Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    SEED.store(x, Ordering::Relaxed);
    x.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

fn pick<T: Copy>(from: &[T]) -> T {
    from[(next() % from.len() as u64) as usize]
}

/// A string of awkward length and content: empty, one character, multi-byte,
/// a control character, or a few kilobytes.
fn text() -> String {
    match next() % 6 {
        0 => String::new(),
        1 => "x".into(),
        2 => "oʻzbek — Ўзбек — 中文 — 🎈".into(),
        3 => "\0\n\t\u{202e}".into(),
        4 => "y".repeat((next() % 4096) as usize),
        _ => format!("{}", next()),
    }
}

fn bytes() -> Vec<u8> {
    let len = (next() % 512) as usize;
    let mut out: Vec<u8> = (0..len).map(|_| next() as u8).collect();
    // Now and then, a PNG signature on the front of the garbage.
    if next() % 3 == 0 {
        out.splice(0..0, *b"\x89PNG\r\n\x1a\n");
    }
    out
}

struct Fuzz;

impl Guest for Fuzz {
    fn init() {
        log("fuzz: init");
    }

    fn update() {
        let frame = FRAME.fetch_add(1, Ordering::Relaxed);
        for _ in 0..40 {
            match next() % 24 {
                0 => log(&text()),
                1 => ui_label(&text()),
                2 => {
                    ui_button(&text());
                }
                3 => {
                    ui_text_edit(&text());
                }
                4 => ui_heading(&text(), pick(&[0, 1, 2, 3, 4, 200, 255])),
                5 => {
                    ui_search_field(&text(), &text());
                }
                6 => {
                    ui_tile(&text(), &text());
                }
                7 => {
                    now_millis();
                }
                8 => {
                    list_apps();
                }
                9 => request_frame(pick(&[0, 1, 16, 1000, u32::MAX])),
                10 => {
                    let _ = storage_get(&text());
                }
                11 => {
                    let _ = storage_set(&text(), &text());
                }
                12 => storage_remove(&text()),
                13 => {
                    storage_keys();
                }
                14 => ui_begin(&text()),
                15 => ui_end(),
                16 => {
                    ui_checkbox(&text(), next() % 2 == 0);
                }
                17 => {
                    let options: Vec<String> = (0..next() % 4).map(|_| text()).collect();
                    ui_choice(&options, pick(&[0, 1, 7, u32::MAX]));
                }
                18 => ui_separator(),
                19 => ui_progress(pick(&[
                    f32::NAN,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    -1.0,
                    0.5,
                    2.0,
                ])),
                20 => {
                    ui_submitted();
                }
                21 => {
                    if let Ok(handle) = image_load(&bytes()) {
                        ui_image(handle);
                    }
                    ui_image(pick(&[0, 1, u32::MAX]));
                }
                22 => {
                    let request = HttpRequest {
                        method: pick(&["GET", "", "DELETE", "NOT A METHOD"]).into(),
                        url: pick(&[
                            "",
                            "http://",
                            "ftp://x",
                            "https://[::1",
                            "https://example.invalid/",
                        ])
                        .into(),
                        headers: vec![(text(), text())],
                        body: bytes(),
                    };
                    let _ = fetch(&request);
                    let _ = fetch_poll(pick(&[0, 1, 99, u32::MAX]));
                }
                _ => {
                    locale();
                }
            }
        }
        ui_label(&format!("fuzz frame {frame}, still here"));
    }
}

export!(Fuzz);
