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
[--input \"frame:field=text\"] [--submit \"frame:field,...\"] [--back \"frame,...\"] [--reload \"frame,...\"] [--confirm] \
[--registry <path-or-url>] [--timing] [--version]\n\
       host --kept | --clear <publisher>/<name> | --clear-cache\n\
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
    let mut reloads = String::new();
    let mut submits = String::new();
    let mut confirm = false;
    let mut timing = false;
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
            // What apps keep on this machine, and the caches (`settings.rs`) —
            // the settings page, for the command line.
            "--kept" => {
                for k in host::settings::kept() {
                    println!("{:<44} {}", k.identity, host::settings::human(k.bytes));
                }
                println!(
                    "cache: {}",
                    host::settings::human(host::settings::cache_bytes())
                );
                return Ok(());
            }
            "--clear" => {
                let identity = args.get(i + 1).cloned().unwrap_or_default();
                host::settings::clear(&identity).map_err(|e| anyhow::anyhow!(e))?;
                println!("cleared what {identity} kept");
                return Ok(());
            }
            "--clear-cache" => {
                host::settings::clear_cache().map_err(|e| anyhow::anyhow!(e))?;
                println!("cleared the cache");
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
            "--reload" => {
                i += 1;
                reloads = args.get(i).cloned().unwrap_or_default();
            }
            "--timing" => timing = true,
            "--submit" => {
                i += 1;
                submits = args.get(i).cloned().unwrap_or_default();
            }
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
            let opts = Harness {
                frames,
                script: &script,
                inputs: &inputs,
                submits: &submits,
                backs: &backs,
                reloads: &reloads,
                confirm,
                timing,
            };
            run_headless(shell, &opts)
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
/// What the headless runner is told to do.
struct Harness<'a> {
    frames: usize,
    script: &'a str,
    inputs: &'a [String],
    submits: &'a str,
    backs: &'a str,
    reloads: &'a str,
    confirm: bool,
    /// Print how long the guest's frames took, after the run.
    timing: bool,
}

fn run_headless(mut shell: Shell, h: &Harness) -> Result<()> {
    let Harness {
        frames,
        script,
        inputs,
        submits,
        backs,
        reloads,
        confirm,
        timing,
    } = *h;
    // Each frame's call into the guest and back, UI collected — the
    // boundary's cost, without a renderer's.
    let mut took = Vec::with_capacity(frames);
    for l in shell.take_logs() {
        println!("[wasm log] {l}");
    }

    let script = parse_script(script);
    // Same shape as a click script: `"2:0"` => at frame 2, field 0 was submitted.
    let submits = parse_script(submits);
    let typed: Vec<(usize, u32, String)> = inputs.iter().filter_map(|s| parse_input(s)).collect();
    // `"4,6"` => press Back before frames 4 and 6 — the system Back key on a
    // phone, the Back button in the window's chrome.
    let reloads: HashSet<usize> = reloads
        .split(',')
        .filter_map(|f| f.trim().parse().ok())
        .collect();
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
        if reloads.contains(&frame) {
            edits.clear();
            shell.reload();
            shell.wait();
            println!("\n[reload] {}", shell.status);
        }
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
        let started = std::time::Instant::now();
        let cmds = shell.frame(FrameInput {
            clicked: clicks.clone(),
            edits: edits.clone(),
            submitted: submits.get(&frame).cloned().unwrap_or_default(),
        })?;
        took.push(started.elapsed());
        // A guest that asked to navigate started a load on another thread.
        // There are no frames to draw while it flies, so block for it here and
        // the run stays exactly as deterministic as it was.
        shell.wait();

        let mut injected: Vec<u32> = clicks.into_iter().collect();
        injected.sort_unstable();
        println!("\n--- frame {frame}  (clicks fed to guest: {injected:?}) ---");

        // Groups indent what is inside them, two spaces a level.
        let mut depth = 0usize;
        for c in &cmds {
            if matches!(c, UiCmd::End) {
                depth = depth.saturating_sub(1);
            }
            let line = match c {
                UiCmd::Label(t) => format!("label   : {t:?}"),
                UiCmd::Button { index, text } => format!("button{index} : {text:?}"),
                UiCmd::TextEdit { index, text } => format!("field{index}  : {text:?}"),
                UiCmd::Heading { text, level } => format!("heading{level} : {text:?}"),
                UiCmd::Search {
                    index,
                    text,
                    placeholder,
                } => format!("search{index} : {text:?}  (hint {placeholder:?})"),
                UiCmd::Tile {
                    index,
                    title,
                    subtitle,
                } => format!("tile{index} : {title:?} {subtitle:?}"),
                UiCmd::Begin { index, kind } => format!("begin {}#{index}", kind.name()),
                UiCmd::End => "end".to_string(),
                UiCmd::Checkbox {
                    index,
                    text,
                    checked,
                } => format!(
                    "check{index} : [{}] {text:?}",
                    if *checked { 'x' } else { ' ' }
                ),
                UiCmd::Choice {
                    index,
                    options,
                    selected,
                } => {
                    let shown: Vec<String> = options
                        .iter()
                        .enumerate()
                        .map(|(i, o)| {
                            if i as u32 == *selected {
                                format!("[{o}]")
                            } else {
                                o.clone()
                            }
                        })
                        .collect();
                    format!("choice{index} : {}", shown.join(" | "))
                }
                UiCmd::Separator => "----".to_string(),
                UiCmd::Progress(f) => format!("progress : {:.0}%", f * 100.0),
                UiCmd::Image(p) => format!("image : {}x{}", p.width, p.height),
            };
            println!("   {}{line}", "  ".repeat(depth));
            if matches!(c, UiCmd::Begin { .. }) {
                depth += 1;
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
        // An app asked to reach an origin: the window would ask the user;
        // here `--confirm` answers. Then its requests are waited for, so the
        // next frame's poll finds them done.
        if let Some((app, origin)) = shell.network_question() {
            println!("   [asks] {app} wants to connect to {origin}");
            shell.answer_network(confirm);
            println!("   [{}]", if confirm { "allowed" } else { "not allowed" });
        }
        shell.settle_network();
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
            let place = shell.location().map(|(title, _)| title.to_string());
            let said = if shell.status.is_empty() {
                format!("now at {}", place.unwrap_or_default())
            } else {
                shell.status.clone()
            };
            println!("   [nav] {said}");
        } else if shell.status_is_error && shell.status != status_before {
            // A guest asked to go somewhere and it did not load. The window
            // shows this in its status bar; without this line the headless run
            // said nothing at all, and the guest just carried on as if asked
            // for nothing.
            println!("   [nav failed] {}", shell.status);
        }
    }

    if timing && !took.is_empty() {
        took.sort_unstable();
        let median = took[took.len() / 2];
        let max = took[took.len() - 1];
        println!(
            "\n[timing] {} frames: median {median:.2?}, max {max:.2?} per frame in the guest",
            took.len()
        );
    }

    Ok(())
}
