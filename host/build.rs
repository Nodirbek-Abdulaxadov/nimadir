//! Bakes the start page into the binary when it has been built, so an installed
//! shell — which has no checkout to read `mini-apps/` from — still opens on the
//! store. A fresh clone has built no components yet; the binary then carries
//! none, and builds exactly as before.

use std::path::{Path, PathBuf};

fn main() {
    let store = Path::new("../mini-apps/store/store.component.wasm");
    println!("cargo:rerun-if-changed={}", store.display());
    println!("cargo:rerun-if-changed=build.rs");

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let bytes = std::fs::read(store).unwrap_or_default();
    std::fs::write(out.join("store.component.wasm"), bytes).expect("write to OUT_DIR");
}
