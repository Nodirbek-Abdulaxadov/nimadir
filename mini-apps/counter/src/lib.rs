//! `counter` mini-app — compiles to `wasm32-unknown-unknown`.
//!
//! It holds its own state (the count) inside its own linear memory. The host
//! knows nothing about "counting"; it only renders the labels/buttons this app
//! describes and routes clicks back. That is the whole point: app logic lives in
//! the sandboxed guest, the host is a generic renderer + capability provider.

use std::sync::atomic::{AtomicI32, Ordering};

// The host's capability table. These are resolved by the host at instantiation
// time (module "env"); here they are just imports.
#[link(wasm_import_module = "env")]
extern "C" {
    fn host_log(ptr: i32, len: i32);
    fn ui_label(ptr: i32, len: i32);
    fn ui_button(ptr: i32, len: i32) -> i32;
    fn host_now_millis() -> i64;
}

// Thin safe wrappers. A `&str`'s bytes live in this module's own linear memory,
// so we just hand the host a (pointer, length) into it — no copy on this side.
fn log(s: &str) {
    unsafe { host_log(s.as_ptr() as i32, s.len() as i32) }
}
fn label(s: &str) {
    unsafe { ui_label(s.as_ptr() as i32, s.len() as i32) }
}
fn button(s: &str) -> bool {
    unsafe { ui_button(s.as_ptr() as i32, s.len() as i32) != 0 }
}

/// App state, owned by the guest.
static COUNT: AtomicI32 = AtomicI32::new(0);

/// Lets the host write `len` bytes into guest memory (part of the ABI). This
/// mini-app doesn't need host->guest data, but the export keeps the ABI complete.
#[no_mangle]
pub extern "C" fn guest_alloc(len: i32) -> i32 {
    let mut buf = Vec::<u8>::with_capacity(len.max(0) as usize);
    let ptr = buf.as_mut_ptr() as i32;
    std::mem::forget(buf);
    ptr
}

/// One-time setup — also exercises a host capability (`host_now_millis`).
#[no_mangle]
pub extern "C" fn init() {
    let now = unsafe { host_now_millis() };
    log(&format!("counter mini-app: init at host-time {now}"));
}

/// Called once per frame. Describe the UI; react to clicks.
#[no_mangle]
pub extern "C" fn update() {
    let count = COUNT.load(Ordering::Relaxed);
    label(&format!("Count: {count}"));

    // Button index 0.
    if button("Increment") {
        COUNT.fetch_add(1, Ordering::Relaxed);
    }
    // Button index 1.
    if button("Reset") {
        COUNT.store(0, Ordering::Relaxed);
    }
}
