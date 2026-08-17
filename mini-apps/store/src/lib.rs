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

use nimadir::shell::host_api::{list_apps, log, open_app, ui_button, ui_label, ui_text_edit};

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

        ui_label("nimadir — apps");

        // Field 0. The value goes out, the edited value comes back.
        let query = {
            let mut q = QUERY.lock().unwrap();
            *q = ui_text_edit(&q);
            q.trim().to_lowercase()
        };

        let matches: Vec<_> = apps
            .iter()
            .filter(|a| {
                query.is_empty()
                    || a.name.to_lowercase().contains(&query)
                    || a.description.to_lowercase().contains(&query)
            })
            .collect();

        if query.is_empty() {
            ui_label(&format!("{} apps", apps.len()));
        } else {
            ui_label(&format!(
                "{} of {} apps matching \"{query}\"",
                matches.len(),
                apps.len()
            ));
        }

        if matches.is_empty() {
            ui_label("Nothing matches. Clear the search to see everything.");
            return;
        }

        for a in matches {
            ui_label(&format!("{} — {}", a.name, a.description));
            ui_label(&format!("    {}", a.source));
            // Naming the button per app matters: the host hands out button
            // indices in call order, so a filtered list renumbers them. The
            // label is what stays meaningful.
            if ui_button(&format!("Open {}", a.name)) {
                log(&format!("store: opening {}", a.name));
                open_app(&a.source);
            }
        }
    }
}

export!(Store);
