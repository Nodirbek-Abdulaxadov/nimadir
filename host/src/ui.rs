//! The tiny, renderer-agnostic UI protocol that a mini-app speaks.
//!
//! A mini-app never touches the screen, the DOM, or a GPU directly. Each frame
//! it *describes* its UI by calling host functions (`ui_label`, `ui_button`).
//! The host collects those calls into a `Vec<UiCmd>` and hands it to whatever
//! backend is active (headless text, or the egui window). This is the seam that
//! lets the SAME mini-app render on any backend — swap the renderer, not the app.

use std::collections::{HashMap, HashSet};

/// One immediate-mode UI element emitted by the guest during a frame.
#[derive(Clone, Debug)]
pub enum UiCmd {
    /// A line of text.
    Label(String),
    /// A clickable button. `index` is assigned by the host in call order, so
    /// the backend can report "button N was clicked" back to the guest.
    Button { index: u32, text: String },
    /// A single-line text field. `text` is the value to display — already the
    /// edited one, since the host answers the guest with last frame's edits.
    TextEdit { index: u32, text: String },
    /// A heading; `level` 1 is the largest, already clamped to 1..=3.
    Heading { text: String, level: u8 },
    /// A prominent search field. A `TextEdit` in every respect that matters —
    /// same index space, same one-frame handshake — drawn larger, with a hint.
    Search {
        index: u32,
        text: String,
        placeholder: String,
    },
    /// A clickable card. The backend lays *runs* of these out as a grid, so a
    /// tile carries no position: only what is on it.
    Tile {
        index: u32,
        title: String,
        subtitle: String,
    },
}

/// Input handed to the guest for a frame.
///
/// Both fields describe what the user did to the *previous* frame's widgets.
/// That one-frame lag is inherent to the design, not an oversight: the guest
/// asks for a widget and gets its result in the same call, so the only result
/// the host can possibly have is the one from the last time it was drawn.
#[derive(Default, Clone)]
pub struct FrameInput {
    /// Button indices clicked.
    pub clicked: HashSet<u32>,
    /// Current contents of each text field, by index. Absent means untouched,
    /// in which case the guest's own value is handed straight back.
    pub edits: HashMap<u32, String>,
}
