//! `probe` — a **test fixture**, not an app. A fixed page, and a field whose
//! text becomes the page's stylesheet, so the suite can try any CSS against
//! the same tree: `--type "1:2=.box { color: red }" --styles`.
//!
//! A second field, `#q`, takes a selector and logs what `query-all` finds
//! for it under the page.
//!
//! Node numbers are fixed by the order `init` makes them, which is the order
//! below; the suite names them.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::cell::Cell;

use nimadir::shell::dom;

thread_local! {
    static SHEET: Cell<u32> = const { Cell::new(0) };
    static PAGE: Cell<u32> = const { Cell::new(0) };
}

fn el(parent: u32, tag: &str, attrs: &[(&str, &str)], text: &str) -> u32 {
    let n = dom::create_element(tag);
    for (k, v) in attrs {
        dom::set_attribute(n, k, v);
    }
    if !text.is_empty() {
        dom::set_text(n, text);
    }
    dom::append_child(parent, n);
    n
}

struct Probe;

impl Guest for Probe {
    fn init() {
        let root = dom::root(); // #1
        let css = el(root, "textarea", &[("id", "css")], ""); // #2
        dom::listen(css, "input");
        let page = el(root, "div", &[("id", "page")], ""); // #3
        let boxed = el(page, "div", &[("id", "a"), ("class", "box wide")], ""); // #4
        el(boxed, "p", &[("class", "first")], "one"); // #5, text #6
        el(boxed, "p", &[], "two"); // #7, #8
        el(boxed, "p", &[("class", "last"), ("lang", "uz-UZ")], "three"); // #9, #10
        let list = el(page, "ul", &[], ""); // #11
        for n in ["1", "2", "3", "4"] {
            el(list, "li", &[], n); // #12 #14 #16 #18, texts between
        }
        el(page, "button", &[], "Go"); // #20, #21
        el(page, "input", &[("type", "checkbox"), ("checked", "")], ""); // #22
        el(
            page,
            "span",
            &[("class", "note"), ("style", "letter-spacing: 1px")],
            "small print",
        ); // #23, #24
        let q = el(root, "input", &[("id", "q")], ""); // #25
        dom::listen(q, "input");
        PAGE.with(|p| p.set(page));
    }

    fn update() {
        for e in dom::events() {
            if e.kind == "input" && dom::get_attribute(e.target, "id").as_deref() == Some("q") {
                let found = dom::query_all(PAGE.with(Cell::get), &e.value);
                nimadir::shell::host_api::log(&format!("query {:?}: {found:?}", e.value));
            } else if e.kind == "input" {
                let old = SHEET.with(Cell::get);
                if old != 0 {
                    dom::remove_stylesheet(old);
                }
                SHEET.with(|s| s.set(dom::add_stylesheet(&e.value)));
            }
        }
    }
}

export!(Probe);
