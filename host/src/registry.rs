//! The shell's "bookmarks" — the list of mini-apps the home screen offers.
//!
//! A browser ships with a start page of links; this is the same idea. Entries
//! are plain data (name, source, description) and the source is exactly what the
//! `--` argument accepts: a local path **or** an http(s) URL. Nothing here knows
//! anything about WASM — it is just an address book.

use std::path::Path;

/// One entry on the home screen.
#[derive(Clone, Debug)]
pub struct AppEntry {
    pub name: String,
    /// File path or http(s) URL of the `.wasm`.
    pub src: String,
    pub description: String,
}

/// The manifest filename looked up next to the working directory.
pub const MANIFEST: &str = "apps.list";

/// Read `apps.list` if present, else fall back to the built-in sample entries.
/// A missing or unreadable manifest is not an error — the shell still opens.
pub fn load(path: &Path) -> Vec<AppEntry> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let entries = parse(&text);
            if entries.is_empty() {
                builtin()
            } else {
                entries
            }
        }
        Err(_) => builtin(),
    }
}

/// Parse the manifest: one entry per line, `name | src | description`.
/// Blank lines and `#` comments are skipped; description is optional.
pub fn parse(text: &str) -> Vec<AppEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('|').map(|p| p.trim());
        let name = parts.next().unwrap_or("").to_string();
        let src = parts.next().unwrap_or("").to_string();
        let description = parts.next().unwrap_or("").to_string();
        if name.is_empty() || src.is_empty() {
            continue;
        }
        out.push(AppEntry {
            name,
            src,
            description,
        });
    }
    out
}

/// Entries used when no manifest exists, so a fresh clone has a working home
/// screen after building the two sample mini-apps.
pub fn builtin() -> Vec<AppEntry> {
    vec![
        AppEntry {
            name: "counter".into(),
            src: "mini-apps/counter/target/wasm32-unknown-unknown/release/counter.wasm".into(),
            description: "A counter; the count lives inside the guest.".into(),
        },
        AppEntry {
            name: "hello".into(),
            src: "mini-apps/hello/target/wasm32-unknown-unknown/release/hello.wasm".into(),
            description: "A second app, to show hot-swap with no host rebuild.".into(),
        },
    ]
}

/// Clean up what a human typed or pasted before it is treated as an address.
///
/// The quote stripping is not cosmetic: Windows Explorer's "Copy as path" puts
/// double quotes around the path, so a straight paste is the *common* case, and
/// `"C:\...\app.wasm"` with the quotes kept is not a filename that exists.
pub fn normalize(input: &str) -> &str {
    let s = input.trim();
    let s = s
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')))
        .unwrap_or(s);
    s.trim()
}

/// Resolve what the user typed to `(source, title)`. A bare registry name (the
/// "bookmark" case) wins; anything else is passed through as a path/URL.
///
/// Both the CLI and the address bar go through here, so typing `counter` means
/// the same thing in either place.
pub fn resolve(input: &str, apps: &[AppEntry]) -> (String, String) {
    let cleaned = normalize(input);
    match apps.iter().find(|e| e.name == cleaned) {
        Some(e) => (e.src.clone(), e.name.clone()),
        None => (cleaned.to_string(), cleaned.to_string()),
    }
}
