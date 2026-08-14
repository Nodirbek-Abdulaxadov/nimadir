//! wasm-shell — a native app shell that loads sandboxed WASM mini-apps on demand
//! and renders their UI natively, with zero web stack (no HTML/CSS/JS/webview).
//!
//!   cargo run --features gui                        # home screen (the "browser")
//!   cargo run --features gui -- counter             # open a listed app directly
//!   cargo run -- <path-or-url.wasm>                 # headless (default)
//!   cargo run -- <path-or-url.wasm> --script "1:0,2:0" --frames 6
//!   cargo run -- --list                             # what the home screen offers
//!
//! With no argument the shell opens its **home screen**: the mini-apps from
//! `apps.list` as links, plus an address bar. Clicking a link fetches that
//! `.wasm` and runs it in the same window; "Home" drops it and goes back.
//!
//! The address bar takes any address, not only components: `resolve.rs`
//! classifies what is there and the shell routes to the matching module. Web
//! pages route to a placeholder for now — the webview that renders them is the
//! next stage of work.
//!
//! The mini-app is untrusted code fetched at runtime; it talks to the host only
//! through the capability table in `host.rs`. Swapping the .wasm runs a
//! different mini-app with no host rebuild — the "browser-like" part.

mod host;
mod registry;
mod resolve;
mod shell;
mod ui;
#[cfg(feature = "gui")]
mod gui;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

use crate::shell::Shell;
use crate::ui::UiCmd;

const USAGE: &str = "usage: host [<address> | <listed-app-name>] \
[--list] [--headless] [--frames N] [--script \"frame:btn,...\"]\n\
       an address is a .wasm component (path or URL) or a web page URL";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut source: Option<String> = None;
    let mut headless = false;
    let mut list = false;
    let mut frames = 8usize;
    let mut script = String::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => headless = true,
            "--list" => list = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
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

    let apps = registry::load(Path::new(registry::MANIFEST));

    if list {
        print_home(&apps);
        return Ok(());
    }

    let engine = wasmtime::Engine::default();
    let mut shell = Shell::new(engine, apps);

    // An argument means "navigate straight there", skipping the home screen. It
    // may be a listed app's name (a bookmark) or any path/URL.
    if let Some(input) = &source {
        let (src, title) = registry::resolve(input, &shell.apps);
        shell.open(&src, &title);
    }

    #[cfg(feature = "gui")]
    if !headless {
        // A failed --  open leaves the shell on home with the error shown there,
        // so the window still comes up. Nothing to do but run.
        return gui::run(shell);
    }

    #[cfg(not(feature = "gui"))]
    let _ = headless; // silence unused warning when gui is disabled

    // Headless: the default, and the verifiable test harness.
    match source {
        // No target: there is no headless home screen to click, so show what the
        // home screen *would* offer.
        None => {
            print_home(&shell.apps);
            println!("\n{USAGE}");
            Ok(())
        }
        Some(_) => {
            if shell.status_is_error {
                anyhow::bail!("{}", shell.status);
            }
            println!("{}", shell.status);
            // A web page has no guest to step: there are no frames to run, and
            // the webview it belongs to is a window backend concern. Routing is
            // still what this verifies — the address was classified, and the
            // shell went somewhere other than a mini-app.
            if shell.is_web_page() {
                return Ok(());
            }
            run_headless(shell, frames, &script)
        }
    }
}

/// Print the registry — the text form of the home screen.
fn print_home(apps: &[registry::AppEntry]) {
    println!("wasm-shell — mini-apps ({}):\n", registry::MANIFEST);
    if apps.is_empty() {
        println!("  (none listed)");
        return;
    }
    for e in apps {
        println!("  {:<10} {}", e.name, e.src);
        if !e.description.is_empty() {
            println!("  {:<10} {}", "", e.description);
        }
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

/// Run the current mini-app for N frames, injecting scripted clicks, printing
/// each frame's UI. No display needed — this is how the boundary is verified.
fn run_headless(mut shell: Shell, frames: usize, script: &str) -> Result<()> {
    for l in shell.take_logs() {
        println!("[wasm log] {l}");
    }

    let script = parse_script(script);
    println!("\n=== headless run: {frames} frames ===");

    for frame in 0..frames {
        let clicks = script.get(&frame).cloned().unwrap_or_default();
        let cmds = shell.frame(clicks.clone())?;

        let mut injected: Vec<u32> = clicks.into_iter().collect();
        injected.sort_unstable();
        println!("\n--- frame {frame}  (clicks fed to guest: {injected:?}) ---");

        for c in &cmds {
            match c {
                UiCmd::Label(t) => println!("   label   : {t:?}"),
                UiCmd::Button { index, text } => println!("   button{index} : {text:?}"),
            }
        }
        for l in shell.take_logs() {
            println!("   [wasm log] {l}");
        }
    }

    Ok(())
}
