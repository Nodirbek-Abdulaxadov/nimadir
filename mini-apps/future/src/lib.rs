//! `future` — a **test fixture**, not an app: a mini-app built against a newer
//! WIT than the host implements (`wit/` here is the real contract plus one
//! function). Opening it must fail with a sentence that says the app needs a
//! newer nimadir, not a linker error.
//!
//! Like `runaway`, it stays out of `apps.json` and is opened by path.

wit_bindgen::generate!({
    world: "mini-app",
    path: "wit",
});

use nimadir::shell::host_api::{from_the_future, now_millis, ui_label};

struct Future;

impl Guest for Future {
    fn init() {}

    fn update() {
        ui_label("Built against a newer contract.");
        // Reachable as far as any compiler can tell, so the import is kept:
        // an unused one would be dropped from the component, and the fixture
        // would test nothing.
        if now_millis() < 0 {
            from_the_future();
        }
    }
}

export!(Future);
