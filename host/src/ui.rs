//! The tiny, renderer-agnostic UI protocol that a mini-app speaks.
//!
//! A mini-app never touches the screen, the DOM, or a GPU directly. Each frame
//! it *describes* its UI by calling host functions (`ui_label`, `ui_button`).
//! The host collects those calls into a `Vec<UiCmd>` and hands it to whatever
//! backend is active (headless text, or the egui window). This is the seam that
//! lets the SAME mini-app render on any backend — swap the renderer, not the app.

use std::collections::HashSet;

/// One immediate-mode UI element emitted by the guest during a frame.
#[derive(Clone, Debug)]
pub enum UiCmd {
    /// A line of text.
    Label(String),
    /// A clickable button. `index` is assigned by the host in call order, so
    /// the backend can report "button N was clicked" back to the guest.
    Button { index: u32, text: String },
}

/// Input handed to the guest for a frame: which button indices were clicked.
#[derive(Default, Clone)]
pub struct FrameInput {
    pub clicked: HashSet<u32>,
}
