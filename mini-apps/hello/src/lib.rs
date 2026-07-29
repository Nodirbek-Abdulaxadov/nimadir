//! `hello` mini-app — a second guest, to prove the host runs whatever .wasm you
//! point it at with no rebuild. Same ABI as `counter`, different behavior.

#[link(wasm_import_module = "env")]
extern "C" {
    fn host_log(ptr: i32, len: i32);
    fn ui_label(ptr: i32, len: i32);
    fn ui_button(ptr: i32, len: i32) -> i32;
}

fn log(s: &str) {
    unsafe { host_log(s.as_ptr() as i32, s.len() as i32) }
}
fn label(s: &str) {
    unsafe { ui_label(s.as_ptr() as i32, s.len() as i32) }
}
fn button(s: &str) -> bool {
    unsafe { ui_button(s.as_ptr() as i32, s.len() as i32) != 0 }
}

#[no_mangle]
pub extern "C" fn guest_alloc(len: i32) -> i32 {
    let mut buf = Vec::<u8>::with_capacity(len.max(0) as usize);
    let ptr = buf.as_mut_ptr() as i32;
    std::mem::forget(buf);
    ptr
}

#[no_mangle]
pub extern "C" fn init() {
    log("hello mini-app: init");
}

#[no_mangle]
pub extern "C" fn update() {
    label("Hello from a *different* mini-app!");
    label("Same host binary, swapped .wasm — no rebuild.");
    if button("Say hi") {
        log("hi from the hello mini-app!");
    }
}
