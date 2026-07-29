//! wasm-shell — a native app shell that loads sandboxed WASM mini-apps on demand
//! and renders their UI natively, with zero web stack (no HTML/CSS/JS/webview).
//!
//!   cargo run -- <path-or-url.wasm>                 # headless (default)
//!   cargo run -- <path-or-url.wasm> --script "1:0,2:0" --frames 6
//!   cargo run --features gui -- <path-or-url.wasm>  # native egui window
//!
//! The mini-app is untrusted code fetched at runtime; it talks to the host only
//! through the capability table in `host.rs`. Swapping the .wasm argument runs a
//! different mini-app with no host rebuild — the "browser-like" part.

mod host;
mod ui;
#[cfg(feature = "gui")]
mod gui;

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};

use crate::ui::UiCmd;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut source: Option<String> = None;
    let mut headless = false;
    let mut frames = 8usize;
    let mut script = String::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => headless = true,
            "--frames" => {
                i += 1;
                frames = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(8);
            }
            "--script" => {
                i += 1;
                script = args.get(i).cloned().unwrap_or_default();
            }
            s if !s.starts_with("--") => source = Some(s.to_string()),
            other => eprintln!("(ignoring unknown arg: {other})"),
        }
        i += 1;
    }

    let source = source.context(
        "usage: host <path-or-url.wasm> [--headless] [--frames N] [--script \"frame:btn,...\"]",
    )?;

    let wasm = load_wasm(&source)?;
    println!("loaded {} bytes of wasm from {}", wasm.len(), source);

    let engine = wasmtime::Engine::default();

    #[cfg(feature = "gui")]
    if !headless {
        return gui::run(engine, wasm, source);
    }

    // Headless: the default, and the verifiable test harness.
    #[cfg(not(feature = "gui"))]
    let _ = headless; // silence unused warning when gui is disabled
    run_headless(&engine, &wasm, frames, &script)
}

/// Fetch the mini-app bytes from a URL (http/https) or a local file path.
fn load_wasm(source: &str) -> Result<Vec<u8>> {
    if source.starts_with("http://") || source.starts_with("https://") {
        use std::io::Read;
        let resp = ureq::get(source)
            .call()
            .with_context(|| format!("GET {source}"))?;
        let mut buf = Vec::new();
        resp.into_reader().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read(source).with_context(|| format!("read file {source}"))
    }
}

/// Parse a click script like `"1:0,2:0,3:0,5:1"` => frame 1 clicks button 0, etc.
fn parse_script(s: &str) -> HashMap<usize, HashSet<u32>> {
    let mut m: HashMap<usize, HashSet<u32>> = HashMap::new();
    for pair in s.split(',').filter(|x| !x.trim().is_empty()) {
        if let Some((f, b)) = pair.split_once(':') {
            if let (Ok(f), Ok(b)) = (f.trim().parse::<usize>(), b.trim().parse::<u32>()) {
                m.entry(f).or_default().insert(b);
            }
        }
    }
    m
}

/// Run the mini-app for N frames, injecting scripted clicks, printing each
/// frame's UI. No display needed — this is how the boundary is verified.
fn run_headless(engine: &wasmtime::Engine, wasm: &[u8], frames: usize, script: &str) -> Result<()> {
    let mut app = host::MiniApp::load(engine, wasm)?;
    for l in app.take_logs() {
        println!("[wasm log] {l}");
    }

    let script = parse_script(script);
    println!("\n=== headless run: {frames} frames ===");

    for frame in 0..frames {
        let clicks = script.get(&frame).cloned().unwrap_or_default();
        let cmds = app.frame(clicks.clone())?;

        let mut injected: Vec<u32> = clicks.into_iter().collect();
        injected.sort_unstable();
        println!("\n--- frame {frame}  (clicks fed to guest: {injected:?}) ---");

        for c in &cmds {
            match c {
                UiCmd::Label(t) => println!("   label   : {t:?}"),
                UiCmd::Button { index, text } => println!("   button{index} : {text:?}"),
            }
        }
        for l in app.take_logs() {
            println!("   [wasm log] {l}");
        }
    }

    Ok(())
}
