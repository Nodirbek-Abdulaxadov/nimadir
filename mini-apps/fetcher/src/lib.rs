//! `fetcher` — a **test fixture**, not an app: fetch the address typed into
//! its field, and show what came back. The suite signs it with a manifest
//! naming the origins it may reach, and drives the grant question (ADR-3).
//!
//! Field 0 is the address, tap 0 fetches.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::sync::Mutex;

use nimadir::shell::host_api::{
    fetch, fetch_poll, locale, ui_button, ui_heading, ui_label, ui_text_edit, HttpRequest,
};

struct State {
    url: String,
    pending: Option<u32>,
    last: String,
}

static STATE: Mutex<State> = Mutex::new(State {
    url: String::new(),
    pending: None,
    last: String::new(),
});

struct Fetcher;

impl Guest for Fetcher {
    fn init() {}

    fn update() {
        let mut s = STATE.lock().unwrap();
        ui_heading("Fetcher", 1);
        ui_label(&format!("locale {}", locale()));

        s.url = ui_text_edit(&s.url);
        if ui_button("Fetch") {
            let request = HttpRequest {
                method: "GET".into(),
                url: s.url.clone(),
                headers: Vec::new(),
                body: Vec::new(),
            };
            match fetch(&request) {
                Ok(id) => s.pending = Some(id),
                Err(why) => s.last = format!("refused: {why}"),
            }
        }

        // Asked every frame until it answers; the host keeps frames coming
        // while a request is out.
        if let Some(id) = s.pending {
            if let Some(answer) = fetch_poll(id) {
                s.pending = None;
                s.last = match answer {
                    Ok(r) => format!("status {} with {} bytes", r.status, r.body.len()),
                    Err(why) => format!("failed: {why}"),
                };
            }
        }

        if s.pending.is_some() {
            ui_label("waiting…");
        } else if !s.last.is_empty() {
            ui_label(&s.last);
        }
    }
}

export!(Fetcher);
