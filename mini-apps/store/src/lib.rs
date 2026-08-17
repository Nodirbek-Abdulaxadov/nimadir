//! `store` — the shell's start page, and the point of the whole exercise.
//!
//! nimadir's home screen used to be host code: a list drawn by `gui.rs` that
//! only the host could change. This replaces it with a **mini-app** — the same
//! sandbox, the same `wit/world.wit`, no privileges the counter does not have.
//! The host stopped owning the start page; it now just runs whichever app is
//! pointed at it, and one of those apps happens to be the catalogue.
//!
//! It needs exactly two capabilities the earlier mini-apps did not:
//!
//! * `list-apps` — a guest can reach neither disk nor network, so the registry
//!   has to be handed to it. What it gets is a snapshot, not a live handle.
//! * `open-app` — a *request*. The host cannot tear this app down while it is
//!   in the middle of `update`, so the navigation happens once this frame
//!   returns. From in here that is invisible: ask, and the next frame belongs
//!   to somebody else.
//!
//! Search is client-side over that snapshot, which is the right place for it
//! while the catalogue is small enough to hand over whole.

use std::sync::Mutex;

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use nimadir::shell::host_api::{
    list_apps, log, open_app, ui_heading, ui_label, ui_search_field, ui_tile,
};

/// The search box's contents. Guest-owned, like every mini-app's state: the
/// host holds the live edit buffer for a frame and hands the value back.
static QUERY: Mutex<String> = Mutex::new(String::new());

struct Store;

impl Guest for Store {
    fn init() {
        log(&format!("store: {} apps in the registry", list_apps().len()));
    }

    fn update() {
        let apps = list_apps();

        // The shape every browser start page has: a wordmark, one big search
        // box, and the things you might open as a grid under it. All three are
        // *described*, never positioned — the host decides what a heading, a
        // search field and a grid of tiles look like on its renderer.
        ui_heading("nimadir", 1);

        // Field 0. The value goes out, the edited value comes back.
        //
        // Two values come out of it: what was typed, and a folded copy to match
        // against. Filtering wants the folded one; opening wants the typed one
        // exactly as typed, because a URL's case is not ours to change.
        let (raw, query) = {
            let mut q = QUERY.lock().unwrap();
            *q = ui_search_field(&q, "Search apps, or type an address");
            let raw = q.trim().to_string();
            let folded = raw.to_lowercase();
            (raw, folded)
        };

        let matches: Vec<_> = apps
            .iter()
            .filter(|a| {
                query.is_empty()
                    || a.name.to_lowercase().contains(&query)
                    || a.description.to_lowercase().contains(&query)
            })
            .collect();

        // Level 3: the search's own feedback, not a section of the page.
        if query.is_empty() {
            ui_heading(&format!("{} apps", apps.len()), 3);
        } else {
            ui_heading(
                &format!(
                    "{} of {} apps matching \"{query}\"",
                    matches.len(),
                    apps.len()
                ),
                3,
            );
        }

        // This field is the only place to type on the start page — the host
        // draws no address bar over it — so it has to accept an address as
        // readily as a search term. Offered as a card rather than acted on
        // silently: there is no Enter key in the UI vocabulary, and guessing
        // that half-typed text is a destination would open pages nobody asked
        // for. Navigation is still the host's: `open-app` is a request, and
        // what turns this text into an address is the same resolver the CLI
        // and the address bar use.
        let typed_address = looks_like_address(&raw);
        if typed_address && ui_tile("Open address", &raw) {
            log(&format!("store: opening typed address {raw}"));
            open_app(&raw);
        }

        if matches.is_empty() {
            if !typed_address {
                ui_label("Nothing matches. Clear the search to see everything.");
            }
            return;
        }

        // One tile per app, in registry order. Tile indices renumber as the
        // list filters — exactly as the buttons did — so what has to stay
        // meaningful is the title on the card, not its position.
        for a in matches {
            if ui_tile(&a.name, &a.description) {
                log(&format!("store: opening {}", a.name));
                open_app(&a.source);
            }
        }
    }
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
