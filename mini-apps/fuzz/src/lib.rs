//! `fuzz` — a **test fixture**, not an app: every function in `host-api`,
//! called each frame with inputs a careless or hostile guest might send —
//! empty and long strings, out-of-range numbers, NaN, garbage for an image,
//! nonsense for a URL, a request number nobody issued. Deterministic: the
//! same seed every run, so a failure can be replayed.
//!
//! The element tree (ADR-4) gets the same: structure edits that would put a
//! node inside itself, numbers that are stale or were never nodes, garbage
//! CSS and markup, selectors that cannot be read — on a page the headless
//! runner styles, lays out and paints after every frame. The fixture keeps
//! under the document's limits on purpose, releasing what it made, so it is
//! the host's answers being tested, not its limits.
//!
//! The host must answer all of it without panicking; the suite runs a few
//! hundred frames and checks the app is still drawing at the end.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use nimadir::shell::dom;
use nimadir::shell::host_api::*;

thread_local! {
    /// The nodes this fixture made and has not released.
    static MADE: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// Its stylesheets.
    static SHEETS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

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
        for _ in 0..40 {
            fuzz_dom();
        }
        ui_label(&format!("fuzz frame {frame}, still here"));
    }
}

/// A node to act on: one it made, the root, or a number that is no node.
fn node() -> u32 {
    let made = MADE.with(|m| {
        let m = m.borrow();
        (!m.is_empty()).then(|| m[(next() % m.len() as u64) as usize])
    });
    match next() % 8 {
        0 => 0,
        1 => 1,
        2 => u32::MAX,
        3 => pick(&[2, 7, 100_000]),
        _ => made.unwrap_or(1),
    }
}

/// CSS a careless sheet might hold: real properties at their edges, cycles
/// of variables, selectors past reading, and plain garbage.
fn css() -> String {
    let selectors = [
        "div",
        "*",
        ".a",
        "#x",
        "a > b ~ c + d",
        ":hover",
        "[x=y]",
        "li:nth-child(2n+1)",
        ":not(.a, #b)",
        "::before",
        "a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a",
        "",
        ")(",
        "div:nth-child(n+)",
    ];
    let decls = [
        "display: grid; grid-template-columns: repeat(auto-fill, minmax(10px, 1fr))",
        "display: flex; flex-wrap: wrap; gap: 1e9px",
        "width: calc(100% - 10px); height: calc((50% + 1px) * 2)",
        "position: absolute; inset: 0; z-index: 99999",
        "float: left; clear: both",
        "font-size: 100000px; line-height: 0",
        "padding: -5px; margin: auto",
        "border: 1e30px solid red; border-radius: 50%",
        "overflow: scroll; max-height: 1px",
        "--x: var(--x); color: var(--x)",
        "--a: var(--b) var(--b); --b: var(--a) var(--a); width: var(--a)",
        "grid-column: span 10000 / -10000; grid-row: 0",
        "background: linear-gradient(1turn, red, blue 200%, green -5%)",
        "box-shadow: 0 0 1e9px red; opacity: NaN",
        "content: 'x'; transform: rotate(9deg)",
        "}{;::",
    ];
    let mut out = String::new();
    for _ in 0..next() % 4 {
        out.push_str(&format!("{} {{ {} }}\n", pick(&selectors), pick(&decls)));
    }
    if next() % 3 == 0 {
        out = format!("@media (max-width: {}px) {{ {out} }}", next() % 2000);
    }
    out
}

fn markup() -> String {
    match next() % 7 {
        0 => "<div class=\"a\"><p>one &amp; two</p><input type=checkbox checked><br></div>".into(),
        1 => "<div><p>unclosed</div>".into(),
        2 => "<ul>".repeat(300) + &"</ul>".repeat(300),
        3 => "<select><option>a<option>b</select>".into(),
        4 => "<x-thing a b=c d='e' f=\"&#xFFFFFF;\">&bogus; &#0;</x-thing>".into(),
        5 => "<script>while(1){}</script><style>* { display: none }</style>".into(),
        _ => text(),
    }
}

/// One call into `dom`, with awkward input.
fn fuzz_dom() {
    // Keep well under the document's limits: past a few thousand nodes, let
    // the oldest go.
    let live = MADE.with(|m| m.borrow().len());
    if live > 2000 {
        let old = MADE.with(|m| m.borrow_mut().drain(..500).collect::<Vec<_>>());
        for n in old {
            dom::release(n);
        }
    }
    let short = || text().chars().take(64).collect::<String>();
    match next() % 22 {
        0 | 1 => {
            let tag = pick(&[
                "div", "span", "input", "select", "option", "img", "textarea", "", "x-y", "UL",
                "p", "a", "button", "ǅ",
            ]);
            let n = dom::create_element(tag);
            MADE.with(|m| m.borrow_mut().push(n));
        }
        2 => {
            let n = dom::create_text(&short());
            MADE.with(|m| m.borrow_mut().push(n));
        }
        3 | 4 => dom::append_child(node(), node()),
        5 => dom::insert_before(node(), node(), node()),
        6 => dom::remove(node()),
        7 => {
            let n = node();
            dom::release(n);
            MADE.with(|m| m.borrow_mut().retain(|&x| x != n));
        }
        8 => dom::set_text(node(), &short()),
        9 => {
            let name = pick(&[
                "class", "id", "style", "type", "src", "checked", "disabled", "hidden", "value",
                "href", "", "x",
            ]);
            let value = match name {
                "style" => pick(&[
                    "width: 50%; display: flex",
                    "color: var(--nope)",
                    "padding: calc(",
                    "",
                ])
                .to_string(),
                "src" => pick(&["0", "1", "4294967295", "x"]).to_string(),
                "type" => pick(&["checkbox", "radio", "range", "RADIO", "nope"]).to_string(),
                _ => short(),
            };
            dom::set_attribute(node(), name, &value);
        }
        10 => dom::remove_attribute(node(), pick(&["class", "style", "value", "checked", ""])),
        11 => {
            let _ = dom::get_attribute(node(), &short());
        }
        12 => dom::toggle_class(node(), pick(&["a", "b c", "", "x"]), next() % 2 == 0),
        13 => dom::set_style(
            node(),
            pick(&["width", "display", "", "--x", "grid-template-columns"]),
            pick(&["10px", "grid", "repeat(3, 1fr)", "", "var(--x)"]),
        ),
        14 => {
            let _ = dom::set_markup(node(), &markup());
        }
        15 => {
            let _ = dom::query_all(
                node(),
                pick(&[
                    "div",
                    "",
                    ")(",
                    ".a > p",
                    "*",
                    "li:nth-child(odd)",
                    "a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a a",
                ]),
            );
        }
        16 => {
            let _ = (
                dom::parent(node()),
                dom::children(node()),
                dom::value(node()),
                dom::bounds(node()),
            );
        }
        17 => dom::focus(node()),
        18 => {
            let id = dom::add_stylesheet(&css());
            SHEETS.with(|s| s.borrow_mut().push(id));
            if SHEETS.with(|s| s.borrow().len()) > 20 {
                let old = SHEETS.with(|s| s.borrow_mut().remove(0));
                dom::remove_stylesheet(old);
            }
        }
        19 => dom::remove_stylesheet(pick(&[0, 1, 3, u32::MAX])),
        20 => {
            let kind = pick(&["click", "input", "", "nope", "CHANGE"]);
            if next() % 2 == 0 {
                dom::listen(node(), kind);
            } else {
                dom::unlisten(node(), kind);
            }
        }
        _ => {
            let _ = dom::events();
        }
    }
}

export!(Fuzz);
