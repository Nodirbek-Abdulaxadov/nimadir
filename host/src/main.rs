//! wasm-shell — a native app shell that loads sandboxed WASM mini-apps on demand
//! and renders their UI natively. No web stack on the mini-app path: HTML, CSS
//! and JS exist only in the optional webview module, for old web pages.
//!
//!   cargo run --features gui                        # the store (the start page)
//!   cargo run --features gui -- counter             # open a listed app directly
//!   cargo run --features webview -- https://…       # a web page, in a webview
//!   cargo run -- <path-or-url.wasm>                 # headless (default)
//!   cargo run -- <path-or-url.wasm> --script "1:0,2:0" --frames 6
//!   cargo run -- --list                             # what the registry offers
//!
//! With no argument the window opens on the **store**: a mini-app that lists
//! and searches the registry, and asks the shell to open whatever you pick. It
//! is an ordinary guest — the start page has no privileges the counter lacks.
//!
//! The address bar takes any address, not only components: `resolve.rs`
//! classifies what is there and the shell routes to the matching module — a
//! component to the sandbox, a web page to the webview.
//!
//! The mini-app is untrusted code fetched at runtime; it talks to the host only
//! through the capability table in `host.rs`. Swapping the .wasm runs a
//! different mini-app with no host rebuild — the "browser-like" part.

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use host::registry;
use host::shell::Shell;
use host::ui::{FrameInput, UiCmd};

const USAGE: &str = "usage: host [<address> | <listed-app-name>] \
[--list] [--headless] [--frames N] [--script \"frame:btn,...\"] \
[--input \"frame:field=text\"] [--back \"frame,...\"] [--confirm] \
[--registry <path-or-url>] [--version]\n\
an address is a .wasm component (path or URL) or a web page URL";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut source: Option<String> = None;
    let mut headless = false;
    let mut list = false;
    let mut frames = 8usize;
    let mut script = String::new();
    // Repeatable rather than comma-separated: typed text is free-form, and a
    // separator inside a search term should not silently truncate it.
    let mut inputs: Vec<String> = Vec::new();
    let mut backs = String::new();
    let mut confirm = false;
    let mut registry_src: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => headless = true,
            "--list" => list = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            "--version" | "-V" => {
                println!(
                    "nimadir {} (WIT {})",
                    env!("CARGO_PKG_VERSION"),
                    host::wit_package()
                );
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
            "--input" => {
                i += 1;
                if let Some(v) = args.get(i) {
                    inputs.push(v.clone());
                }
            }
            "--confirm" => confirm = true,
            "--back" => {
                i += 1;
                backs = args.get(i).cloned().unwrap_or_default();
            }
            "--registry" => {
                i += 1;
                registry_src = args.get(i).cloned();
            }
            s if !s.starts_with("--") => source = Some(s.to_string()),
            other => eprintln!("(ignoring unknown arg: {other})"),
        }
        i += 1;
    }

    host::diag::init();

    // Where the catalogue came from, so `--list` can say so instead of naming
    // a manifest that may not have been the one read.
    let (mut apps, mut registry_label, refresh) = match &registry_src {
        Some(s) => {
            let (apps, label) = registry::load_from(s);
            (apps, label, None)
        }
        None => registry::load_default(),
    };
    registry::ensure_start_page(&mut apps);

    if list {
        // Nothing to draw while it arrives, so `--list` waits for the fresh
        // copy rather than printing a stale one.
        if let Some(fresh) = refresh.as_deref().and_then(registry::fetch_hosted) {
            apps = fresh;
            registry::ensure_start_page(&mut apps);
            registry_label = refresh.unwrap_or_default();
        }
        print_home(&apps, &registry_label);
        return Ok(());
    }

    let mut shell = Shell::new(host::engine()?, apps);
    if let Some(url) = refresh {
        shell.refresh_registry(url);
        // The window opens on what it has and installs the rest when it
        // lands. The headless runner has no frames to wait through.
        #[cfg(feature = "gui")]
        let windowed = !headless;
        #[cfg(not(feature = "gui"))]
        let windowed = false;
        if !windowed {
            shell.wait_registry();
        }
    }

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
        return host::gui::run(shell);
    }

    #[cfg(not(feature = "gui"))]
    let _ = headless; // silence unused warning when gui is disabled

    // Headless: the default, and the verifiable test harness.
    match source {
        // No target: there is no headless home screen to click, so show what the
        // home screen *would* offer.
        None => {
            print_home(&shell.apps, &registry_label);
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
            run_headless(shell, frames, &script, &inputs, &backs, confirm)
        }
    }
}

/// Print the registry — the text form of the home screen.
fn print_home(apps: &[registry::AppEntry], source: &str) {
    println!("wasm-shell — mini-apps ({source}):\n");
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

/// Parse one `--input` value, `"frame:field=text"` => at that frame, that text
/// field contains that text. Everything after the first `=` is the value, so a
/// search term may contain anything at all.
fn parse_input(s: &str) -> Option<(usize, u32, String)> {
    let (when, value) = s.split_once('=')?;
    let (frame, field) = when.split_once(':')?;
    Some((
        frame.trim().parse().ok()?,
        field.trim().parse().ok()?,
        value.to_string(),
    ))
}

/// Run the current mini-app for N frames, injecting scripted clicks, printing
/// each frame's UI. No display needed — this is how the boundary is verified.
fn run_headless(
    mut shell: Shell,
    frames: usize,
    script: &str,
    inputs: &[String],
    backs: &str,
    confirm: bool,
) -> Result<()> {
    for l in shell.take_logs() {
        println!("[wasm log] {l}");
    }

    let script = parse_script(script);
    let typed: Vec<(usize, u32, String)> = inputs.iter().filter_map(|s| parse_input(s)).collect();
    // `"4,6"` => press Back before frames 4 and 6 — the system Back key on a
    // phone, the Back button in the window's chrome.
    let backs: HashSet<usize> = backs
        .split(',')
        .filter_map(|f| f.trim().parse().ok())
        .collect();

    // Text fields keep their contents between frames, the way the window
    // backend's live buffers do; `--input` sets a value at a frame and it
    // stays set afterwards.
    let mut edits: HashMap<u32, String> = HashMap::new();

    println!("\n=== headless run: {frames} frames ===");

    for frame in 0..frames {
        if backs.contains(&frame) {
            // The page left behind takes its text fields with it, as in the
            // window: what was typed belonged to an instance that is gone.
            edits.clear();
            if shell.back() {
                shell.wait();
                match shell.location() {
                    Some((title, _)) => println!("\n[back] now at {title}"),
                    None => println!("\n[back] {}", shell.status),
                }
            } else {
                println!("\n[back] nothing to go back to");
            }
        }

        for (_, field, value) in typed.iter().filter(|(f, _, _)| *f == frame) {
            edits.insert(*field, value.clone());
        }

        let clicks = script.get(&frame).cloned().unwrap_or_default();
        // To tell a navigation that failed during this frame from an error
        // that was already showing before it.
        let status_before = shell.status.clone();
        let cmds = shell.frame(FrameInput {
            clicked: clicks.clone(),
            edits: edits.clone(),
        })?;
        // A guest that asked to navigate started a load on another thread.
        // There are no frames to draw while it flies, so block for it here and
        // the run stays exactly as deterministic as it was.
        shell.wait();

        let mut injected: Vec<u32> = clicks.into_iter().collect();
        injected.sort_unstable();
        println!("\n--- frame {frame}  (clicks fed to guest: {injected:?}) ---");

        for c in &cmds {
            match c {
                UiCmd::Label(t) => println!("   label   : {t:?}"),
                UiCmd::Button { index, text } => println!("   button{index} : {text:?}"),
                UiCmd::TextEdit { index, text } => println!("   field{index}  : {text:?}"),
                UiCmd::Heading { text, level } => println!("   heading{level} : {text:?}"),
                UiCmd::Search {
                    index,
                    text,
                    placeholder,
                } => println!("   search{index} : {text:?}  (hint {placeholder:?})"),
                UiCmd::Tile {
                    index,
                    title,
                    subtitle,
                } => println!("   tile{index} : {title:?} {subtitle:?}"),
            }
        }
        for l in shell.take_logs() {
            println!("   [wasm log] {l}");
        }
        // The window would wait this long before drawing again unasked; here
        // frames come at once, so it is only reported.
        if let Some(ms) = shell.take_frame_request() {
            println!("   [next frame in {ms} ms]");
        }
        // An app asked to open somewhere unlisted: the window would ask the
        // user; here `--confirm` answers yes, and its absence no.
        if let Some(ask) = shell.asking() {
            println!("   [asks] {} wants to open {}", ask.from, ask.address);
        }
        if shell.asking().is_some() {
            shell.answer(confirm);
            shell.wait();
            println!("   [{}]", if confirm { "opened" } else { "declined" });
        }
        // A guest that asked to navigate has already been replaced by the app
        // it asked for; say so, or the next frame's different UI looks like a
        // glitch.
        if shell.took_navigation() {
            println!("   [nav] {}", shell.status);
        } else if shell.status_is_error && shell.status != status_before {
            // A guest asked to go somewhere and it did not load. The window
            // shows this in its status bar; without this line the headless run
            // said nothing at all, and the guest just carried on as if asked
            // for nothing.
            println!("   [nav failed] {}", shell.status);
        }
    }

    Ok(())
}
