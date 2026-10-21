//! `store` — the shell's start page, and the point of the whole exercise.
//!
//! nimadir's home screen used to be host code: a list drawn by `gui.rs` that
//! only the host could change. This replaces it with a **mini-app** — the same
//! sandbox, the same `wit/world.wit`, no privileges the counter does not have.
//! The host stopped owning the start page; it now just runs whichever app is
//! pointed at it, and one of those apps happens to be the catalogue.
//!
//! It is built on the element tree (ADR-4), the way a web page is: markup and
//! a stylesheet, built once; events answered by editing only what changes.
//! Each app is a card — an `<a href>` the shell follows when tapped — and the
//! search hides the cards that do not match rather than rebuilding them, so a
//! card keeps its node from one keystroke to the next.
//!
//! What it needs that other apps do not: `list-apps`, because a guest can
//! reach neither disk nor network, so the registry is handed to it.

use std::cell::RefCell;

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use nimadir::shell::dom;
use nimadir::shell::host_api::{list_apps, log, open_app, AppEntry};

const STYLE: &str = "
.home { max-width: 880px; margin: 0 auto; padding: 48px 16px 24px;
        display: flex; flex-direction: column; align-items: center; gap: 16px; }
.wordmark { font-size: 44px; margin: 0; letter-spacing: -1px; }
#q { width: 100%; max-width: 560px; padding: 12px 18px; border-radius: 24px; }
.count { margin: 0; color: var(--nimadir-muted); font-size: 13px; }
.grid { width: 100%; display: grid; gap: 12px;
        grid-template-columns: repeat(auto-fill, minmax(196px, 1fr)); }
.tile { display: flex; flex-direction: column; gap: 6px; padding: 14px;
        border-radius: 12px; background: var(--nimadir-surface);
        border: 1px solid transparent; color: var(--nimadir-text);
        text-decoration: none; cursor: pointer; }
.tile:hover { border-color: var(--nimadir-border); background: var(--nimadir-accent-wash); }
.tile:active { border-color: var(--nimadir-accent); }
.badge { width: 32px; height: 32px; border-radius: 50%; display: flex;
         align-items: center; justify-content: center; color: #fff;
         font-weight: 700; font-size: 15px; }
.name { font-weight: 600; }
.about { color: var(--nimadir-muted); font-size: 13px; }
.open { width: 100%; max-width: 560px; }
.open .badge { background: var(--nimadir-accent); }
.empty { color: var(--nimadir-muted); margin: 8px 0; }
.settings { color: var(--nimadir-muted); font-size: 13px; }
";

/// One per app badge, by its name: colour is decoration, and also the
/// fastest way to tell two cards apart before reading either.
const BADGES: [&str; 8] = [
    "#1a73e8", "#ea4335", "#34a853", "#f9ab00", "#a142f4", "#12b5cb", "#fa7b17", "#e52592",
];

/// What the page keeps of its own nodes.
struct Page {
    search: u32,
    count: u32,
    grid: u32,
    open: u32,
    open_label: u32,
    empty: u32,
    /// The registry the cards were built from, and each card's node.
    apps: Vec<AppEntry>,
    cards: Vec<u32>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
}

struct Store;

impl Guest for Store {
    fn init() {
        let apps = list_apps();
        log(&format!("store: {} apps in the registry", apps.len()));
        dom::add_stylesheet(STYLE);
        let root = dom::root();
        let _ = dom::set_markup(
            root,
            "<main class=\"home\">\
             <h1 class=\"wordmark\">nimadir</h1>\
             <input type=\"search\" id=\"q\" placeholder=\"Search apps, or type an address\">\
             <p class=\"count\"></p>\
             <a class=\"tile open\" id=\"open\" hidden><span class=\"badge\" aria-hidden=\"true\">&gt;</span>\
             <span class=\"name\">Open address</span><span class=\"about\"></span></a>\
             <div class=\"grid\"></div>\
             <p class=\"empty\" hidden></p>\
             <a class=\"settings\" href=\"nimadir:settings\">Settings</a>\
             </main>",
        );
        let one = |sel: &str| dom::query_all(root, sel).first().copied().unwrap_or(0);
        let mut page = Page {
            search: one("#q"),
            count: one(".count"),
            grid: one(".grid"),
            open: one("#open"),
            open_label: one("#open .about"),
            empty: one(".empty"),
            apps: Vec::new(),
            cards: Vec::new(),
        };
        dom::listen(page.search, "input");
        dom::listen(page.search, "submit");
        page.build(apps);
        page.filter();
        PAGE.with(|p| *p.borrow_mut() = Some(page));
    }

    fn update() {
        let events = dom::events();
        PAGE.with(|p| {
            let mut p = p.borrow_mut();
            let Some(page) = p.as_mut() else { return };
            // A fresher registry can arrive while the page is open.
            let apps = list_apps();
            if !same(&apps, &page.apps) {
                page.build(apps);
                page.filter();
            }
            for e in events {
                match e.kind.as_str() {
                    "input" => page.filter(),
                    "submit" => page.go(),
                    _ => {}
                }
            }
        });
    }
}

impl Page {
    /// A card for each app, in registry order.
    fn build(&mut self, apps: Vec<AppEntry>) {
        let mut markup = String::new();
        for a in &apps {
            let initial = a
                .name
                .chars()
                .next()
                .map_or('?', |c| c.to_ascii_uppercase());
            markup.push_str(&format!(
                "<a class=\"tile\" href=\"{}\"><span class=\"badge\" aria-hidden=\"true\" style=\"background: {}\">{}</span>\
                 <span class=\"name\">{}</span><span class=\"about\">{}</span></a>",
                escape(&a.source),
                badge(&a.name),
                escape(&initial.to_string()),
                escape(&a.name),
                escape(&a.description),
            ));
        }
        if let Err(why) = dom::set_markup(self.grid, &markup) {
            log(&format!("store: could not build the cards: {why}"));
        }
        self.cards = dom::children(self.grid);
        self.apps = apps;
    }

    /// Show the cards that match what is typed, and say how many.
    fn filter(&self) {
        let raw = dom::value(self.search);
        let raw = raw.trim();
        let query = raw.to_lowercase();
        let mut shown = 0;
        for (a, &card) in self.apps.iter().zip(&self.cards) {
            let hit = query.is_empty()
                || a.name.to_lowercase().contains(&query)
                || a.description.to_lowercase().contains(&query);
            show(card, hit);
            shown += hit as usize;
        }
        let total = self.apps.len();
        let line = if query.is_empty() {
            format!("{total} apps")
        } else {
            format!("{shown} of {total} apps matching \"{raw}\"")
        };
        dom::set_text(self.count, &line);

        // This field is the only place to type on the start page — the host
        // draws no address bar over it — so it has to take an address as
        // readily as a search. Offered as a card, never acted on while typing.
        let address = looks_like_address(raw);
        show(self.open, address);
        if address {
            dom::set_attribute(self.open, "href", raw);
            dom::set_text(self.open_label, raw);
        }

        let empty = if total == 0 {
            // A registry not fetched yet, or none to fetch: clearing the
            // search would change nothing, so do not suggest it.
            Some("No apps are listed yet. Type an address above to open one.")
        } else if shown == 0 && !address {
            Some("Nothing matches. Clear the search to see everything.")
        } else {
            None
        };
        show(self.empty, empty.is_some());
        if let Some(text) = empty {
            dom::set_text(self.empty, text);
        }
    }

    /// Enter: the typed address, or the one app that matches.
    fn go(&self) {
        let raw = dom::value(self.search);
        let raw = raw.trim();
        if looks_like_address(raw) {
            log(&format!("store: opening typed address {raw}"));
            open_app(raw);
            return;
        }
        let matching: Vec<&AppEntry> = self
            .apps
            .iter()
            .zip(&self.cards)
            .filter(|(_, &card)| dom::get_attribute(card, "hidden").is_none())
            .map(|(a, _)| a)
            .collect();
        if let [only] = matching.as_slice() {
            log(&format!("store: opening {}", only.name));
            open_app(&only.source);
        }
    }
}

fn show(node: u32, visible: bool) {
    if visible {
        dom::remove_attribute(node, "hidden");
    } else {
        dom::set_attribute(node, "hidden", "");
    }
}

fn same(a: &[AppEntry], b: &[AppEntry]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.name == y.name && x.source == y.source && x.description == y.description
        })
}

/// Stable per name, so a card keeps its colour across visits.
fn badge(name: &str) -> &'static str {
    let hash = name
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    BADGES[hash as usize % BADGES.len()]
}

/// Text made safe to put in markup: the registry is data, not markup.
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Whether typed text is plausibly an address rather than a search term.
///
/// Deliberately loose, and deliberately not the real rule: the host's resolver
/// is what decides what an address means, and it reports a bad one properly.
/// All this has to do is know when to *offer* — so a separator of some kind is
/// enough, and a bare word stays a search.
fn looks_like_address(s: &str) -> bool {
    s.contains("://") || s.contains('/') || s.contains('\\') || s.contains('.')
}

export!(Store);
