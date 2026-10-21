//! `todo` — a to-do list built on the element tree (ADR-4), the way a web
//! page would be: elements, classes and a stylesheet, events read in `update`.
//!
//! Nothing here is redrawn each frame. `init` builds the page once; `update`
//! reads what the user did and edits only the nodes that change — a new `li`,
//! a class toggled, a line of text. The host keeps the tree and draws it.

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use nimadir::shell::dom::{self, Event};
use nimadir::shell::host_api::log;

const CSS: &str = "
.app { max-width: 560px; margin: 0 auto; padding: 24px 16px;
       display: flex; flex-direction: column; gap: 16px; }
h1 { font-size: 28px; margin: 0; }
.new { display: flex; gap: 8px; }
.new input { flex: 1; }
.list { list-style: none; padding: 0; margin: 0;
        display: flex; flex-direction: column; gap: 8px; }
.item { display: flex; align-items: center; gap: 12px; padding: 12px 14px;
        border-radius: 10px; background: var(--nimadir-surface);
        border: 1px solid var(--nimadir-border); }
.item span { flex: 1; }
.item.done span { color: var(--nimadir-muted); text-decoration: line-through; }
.delete { background: transparent; border: none; color: var(--nimadir-muted); }
.footer { color: var(--nimadir-muted); font-size: 13px; margin: 0; }
";

struct Item {
    li: u32,
    check: u32,
    delete: u32,
    done: bool,
}

struct Page {
    field: u32,
    add: u32,
    list: u32,
    footer: u32,
    items: Vec<Item>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
}

/// An element with a class, under `parent`.
fn el(parent: u32, tag: &str, class: &str) -> u32 {
    let n = dom::create_element(tag);
    if !class.is_empty() {
        dom::set_attribute(n, "class", class);
    }
    dom::append_child(parent, n);
    n
}

struct Todo;

impl Guest for Todo {
    fn init() {
        log("todo: building the page");
        dom::add_stylesheet(CSS);

        let app = el(dom::root(), "div", "app");
        dom::set_text(el(app, "h1", ""), "To do");
        let row = el(app, "div", "new");
        let field = el(row, "input", "");
        dom::set_attribute(field, "placeholder", "What needs doing?");
        let add = el(row, "button", "add");
        dom::set_text(add, "Add");
        let list = el(app, "ul", "list");
        let footer = el(app, "p", "footer");

        // One listener on the list hears every item's checkbox and button:
        // an event goes to the nearest listening ancestor, naming the node
        // it happened on.
        dom::listen(field, "submit");
        dom::listen(add, "click");
        dom::listen(list, "click");
        dom::listen(list, "change");

        let mut page = Page {
            field,
            add,
            list,
            footer,
            items: Vec::new(),
        };
        page.add_item("Try the element tree");
        page.add_item("Style it with CSS");
        page.count();
        PAGE.with(|p| *p.borrow_mut() = Some(page));
    }

    fn update() {
        let events = dom::events();
        if events.is_empty() {
            return;
        }
        PAGE.with(|p| {
            if let Some(page) = p.borrow_mut().as_mut() {
                for e in events {
                    page.handle(&e);
                }
                page.count();
            }
        });
    }
}

impl Page {
    fn add_item(&mut self, text: &str) {
        let li = el(self.list, "li", "item");
        let check = el(li, "input", "");
        dom::set_attribute(check, "type", "checkbox");
        dom::set_text(el(li, "span", ""), text);
        let delete = el(li, "button", "delete");
        dom::set_text(delete, "×");
        self.items.push(Item {
            li,
            check,
            delete,
            done: false,
        });
    }

    fn handle(&mut self, e: &Event) {
        match e.kind.as_str() {
            "submit" => self.add_from_field(),
            "click" if e.current == self.add => self.add_from_field(),
            "click" => {
                if let Some(at) = self.items.iter().position(|i| within(e.target, i.delete)) {
                    dom::release(self.items.remove(at).li);
                }
            }
            "change" => {
                if let Some(item) = self.items.iter_mut().find(|i| i.check == e.target) {
                    item.done = e.value == "true";
                    dom::toggle_class(item.li, "done", item.done);
                }
            }
            _ => {}
        }
    }

    fn add_from_field(&mut self) {
        let text = dom::value(self.field);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.add_item(text);
        dom::set_attribute(self.field, "value", "");
        dom::focus(self.field);
    }

    fn count(&self) {
        let left = self.items.iter().filter(|i| !i.done).count();
        let line = match (self.items.len(), left) {
            (0, _) => "Nothing to do.".to_string(),
            (_, 0) => "All done.".to_string(),
            (_, 1) => "1 thing left".to_string(),
            (_, n) => format!("{n} things left"),
        };
        dom::set_text(self.footer, &line);
    }
}

/// Whether `node` is `ancestor` or somewhere under it.
fn within(mut node: u32, ancestor: u32) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match dom::parent(node) {
            Some(p) => node = p,
            None => return false,
        }
    }
}

export!(Todo);
