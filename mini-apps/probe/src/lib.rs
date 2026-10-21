//! `probe` — a **test fixture**, not an app. A fixed page, and a field whose
//! text becomes the page's stylesheet, so the suite can try any CSS against
//! the same tree: `--type "1:2=.box { color: red }" --styles`.
//!
//! A second field, `#q`, takes a selector and logs what `query-all` finds
//! for it under the page; the button logs where `#a` was laid out
//! (`bounds`).
//!
//! Below them a form — a label, a field, a checkbox in a label, a select, a
//! range, a submit button, a disabled field — logs every `input`, `change`
//! and `submit` it hears.
//!
//! Last, a field (`#markup`) whose text becomes the markup of the `div`
//! before it (`set-markup`), logging why when it cannot be read.
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

/// The form's node number, and the markup field's, by the order `init`
/// makes things.
const FORM: u32 = 26;
const MARKUP: u32 = 45;

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
        let go = el(page, "button", &[], "Go"); // #20, #21
        dom::listen(go, "click");
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

        let form = el(root, "form", &[("id", "f")], ""); // #26
        el(form, "label", &[("for", "name")], "Name"); // #27, #28
        el(form, "input", &[("id", "name")], ""); // #29
        let agree = el(form, "label", &[], ""); // #30
        el(agree, "input", &[("type", "checkbox")], ""); // #31
        let words = dom::create_text(" Agree"); // #32
        dom::append_child(agree, words);
        let select = el(form, "select", &[("id", "color")], ""); // #33
        el(select, "option", &[("value", "r")], "red"); // #34, #35
        el(select, "option", &[("value", "g")], "green"); // #36, #37
        el(
            select,
            "option",
            &[("value", "b"), ("selected", "")],
            "blue",
        ); // #38, #39
        let range = [
            ("type", "range"),
            ("min", "0"),
            ("max", "10"),
            ("step", "2"),
            ("value", "4"),
        ];
        el(form, "input", &range, ""); // #40
        el(form, "button", &[], "Send"); // #41, #42
        el(form, "input", &[("disabled", ""), ("value", "locked")], ""); // #43
        for kind in ["input", "change", "submit"] {
            dom::listen(form, kind);
        }
        el(root, "div", &[("id", "out")], ""); // #44
        let markup = el(root, "textarea", &[("id", "markup")], ""); // #45
        dom::listen(markup, "input");
    }

    fn update() {
        for e in dom::events() {
            if e.current == MARKUP {
                if let Err(why) = dom::set_markup(MARKUP - 1, &e.value) {
                    nimadir::shell::host_api::log(&format!("markup: {why}"));
                }
            } else if e.current == FORM {
                let line = format!("form {} #{} {:?}", e.kind, e.target, e.value);
                nimadir::shell::host_api::log(&line);
            } else if e.kind == "click" {
                nimadir::shell::host_api::log(&format!("bounds {:?}", dom::bounds(4)));
            } else if e.kind == "input"
                && dom::get_attribute(e.target, "id").as_deref() == Some("q")
            {
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
