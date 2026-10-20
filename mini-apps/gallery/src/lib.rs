//! `gallery` mini-app — every widget the shell draws, each showing what it
//! answers back. A reference for writing apps, and what the test suite drives
//! to check the vocabulary end to end.
//!
//! Index spaces, since the headless scripts depend on them: taps (buttons,
//! tiles, checkboxes) count 0, 1, 2… in call order; values (text fields,
//! search fields, choices) count separately.

wit_bindgen::generate!({
    world: "mini-app",
    path: "../../wit",
});

use std::sync::Mutex;

use nimadir::shell::host_api::{
    image_load, log, ui_begin, ui_button, ui_checkbox, ui_choice, ui_end, ui_heading, ui_image,
    ui_label, ui_progress, ui_separator, ui_submitted, ui_text_edit,
};

/// Guest-owned state, like every mini-app's.
struct State {
    taps: u32,
    remember: bool,
    name: String,
    greeted: Option<String>,
    size: u32,
    /// The badge, as `image-load` numbered it.
    badge: Option<u32>,
}

static STATE: Mutex<State> = Mutex::new(State {
    taps: 0,
    remember: false,
    name: String::new(),
    greeted: None,
    size: 0,
    badge: None,
});

const SIZES: [&str; 3] = ["Small", "Medium", "Large"];

struct Gallery;

impl Guest for Gallery {
    /// The image is handed over once; every frame after draws it by number.
    fn init() {
        match image_load(include_bytes!("../assets/badge.png")) {
            Ok(handle) => STATE.lock().unwrap().badge = Some(handle),
            Err(why) => log(&format!("gallery: no badge: {why}")),
        }
    }

    fn update() {
        let mut s = STATE.lock().unwrap();

        ui_heading("Gallery", 1);
        ui_label("Every widget the shell draws, and what each answers back.");

        ui_begin("section");
        ui_heading("Taps", 2);
        ui_begin("row");
        // Tap 0.
        if ui_button("Tap me") {
            s.taps += 1;
        }
        ui_label(&format!("tapped {} times", s.taps));
        ui_end();
        // Tap 1.
        s.remember = ui_checkbox("Remember me", s.remember);
        ui_label(if s.remember {
            "Remembering."
        } else {
            "Not remembering."
        });
        ui_end();

        ui_begin("section");
        ui_heading("Text", 2);
        // Value 0. Greets on submit — Enter, or the keyboard's action key —
        // not on every keystroke.
        s.name = ui_text_edit(&s.name);
        if ui_submitted() {
            s.greeted = Some(s.name.trim().to_string());
        }
        match &s.greeted {
            Some(name) if !name.is_empty() => ui_label(&format!("Hello, {name}!")),
            _ => ui_label("Type a name and press Enter."),
        }
        ui_end();

        ui_begin("section");
        ui_heading("Choices", 2);
        // Value 1.
        let options: Vec<String> = SIZES.iter().map(|o| o.to_string()).collect();
        s.size = ui_choice(&options, s.size);
        ui_label(&format!("Size: {}", SIZES[s.size as usize]));
        ui_progress((s.size + 1) as f32 / SIZES.len() as f32);
        ui_end();

        ui_begin("section");
        ui_heading("Images", 2);
        if let Some(badge) = s.badge {
            ui_image(badge);
        }
        ui_end();

        ui_separator();

        ui_heading("A list that scrolls", 3);
        ui_begin("scroll");
        for n in 1..=12 {
            ui_label(&format!("Row {n}"));
        }
        ui_end();
    }
}

export!(Gallery);
