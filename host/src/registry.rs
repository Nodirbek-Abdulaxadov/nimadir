//! The shell's "bookmarks" — the list of mini-apps it knows about.
//!
//! A browser ships with a start page of links; this is the same idea. Entries
//! are plain data (name, source, description) and the source is exactly what the
//! `--` argument accepts: a local path **or** an http(s) URL. Nothing here knows
//! anything about WASM — it is just an address book.
//!
//! Two formats, in precedence order: `apps.json`, then `apps.list`. JSON is
//! what a registry *server* would serve, so the same parser handles a local
//! catalogue and a remote one; the pipe-delimited `apps.list` stays because it
//! is pleasant to hand-edit and was here first. A registry can also be a URL,
//! which is the whole of "remote registry" — the catalogue moves off disk and
//! nothing else changes.

use std::path::Path;

use serde::Deserialize;

/// One entry on the home screen.
#[derive(Clone, Debug, Deserialize)]
pub struct AppEntry {
    pub name: String,
    /// File path or http(s) URL of the `.wasm`.
    #[serde(alias = "source")]
    pub src: String,
    #[serde(default)]
    pub description: String,
}

/// The JSON catalogue, looked up first.
pub const MANIFEST_JSON: &str = "apps.json";
/// The original pipe-delimited manifest, still supported.
pub const MANIFEST: &str = "apps.list";

/// Load the registry from a URL (`http(s)://…`, always JSON) or a local file
/// (`.json` by extension, else the pipe format).
///
/// Every failure falls back rather than propagating: a shell that will not open
/// because its bookmarks file is malformed is worse than one that opens with
/// the built-in list and says so.
/// Returns the entries and a label for where they *actually* came from —
/// which is not always the source asked for, since every failure falls back.
/// Saying "read from <url>" after failing to reach it would be a lie the user
/// then has to debug around.
pub fn load_from(source: &str) -> (Vec<AppEntry>, String) {
    let text = if source.starts_with("http://") || source.starts_with("https://") {
        fetch(source)
    } else {
        std::fs::read_to_string(source).ok()
    };

    let entries = text
        .map(|t| {
            if source.ends_with(".json") {
                parse_json(&t)
            } else {
                parse(&t)
            }
        })
        .unwrap_or_default();

    if entries.is_empty() {
        (builtin(), format!("built-in — {source} was unusable"))
    } else {
        (entries, source.to_string())
    }
}

fn fetch(url: &str) -> Option<String> {
    ureq::get(url).call().ok()?.into_string().ok()
}

/// Read the default manifests: `apps.json`, then `apps.list`, then the built-in
/// entries. A missing manifest is not an error — the shell still opens.
pub fn load(dir: &Path) -> Vec<AppEntry> {
    let json = dir.join(MANIFEST_JSON);
    if json.is_file() {
        let entries = std::fs::read_to_string(&json)
            .ok()
            .map(|t| parse_json(&t))
            .unwrap_or_default();
        if !entries.is_empty() {
            return entries;
        }
    }
    match std::fs::read_to_string(dir.join(MANIFEST)) {
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

/// Name whichever default manifest is actually present, for messages that
/// would otherwise claim to have read a file they did not.
pub fn describe_default() -> String {
    if Path::new(MANIFEST_JSON).is_file() {
        MANIFEST_JSON.to_string()
    } else if Path::new(MANIFEST).is_file() {
        MANIFEST.to_string()
    } else {
        "built-in".to_string()
    }
}

/// Parse the JSON catalogue: either a bare array of entries, or an object with
/// an `apps` array, since a real registry server tends to wrap its payload.
pub fn parse_json(text: &str) -> Vec<AppEntry> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Catalogue {
        Bare(Vec<AppEntry>),
        Wrapped { apps: Vec<AppEntry> },
    }

    match serde_json::from_str::<Catalogue>(text) {
        Ok(Catalogue::Bare(apps)) | Ok(Catalogue::Wrapped { apps }) => apps
            .into_iter()
            .filter(|e| !e.name.is_empty() && !e.src.is_empty())
            .collect(),
        Err(_) => Vec::new(),
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
            src: "mini-apps/counter/counter.component.wasm".into(),
            description: "A counter; the count lives inside the guest.".into(),
        },
        AppEntry {
            name: "hello".into(),
            src: "mini-apps/hello/hello.component.wasm".into(),
            description: "A second app, to show hot-swap with no host rebuild.".into(),
        },
        AppEntry {
            name: "counter-cs".into(),
            src: "mini-apps/counter-cs/counter-cs.component.wasm".into(),
            description: "The counter written in C# (.NET NativeAOT) — same WIT, no host change.".into(),
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

/// Resolve what the user typed to `(source, title)`.
///
/// Both the CLI and the address bar go through here, so typing `counter` means
/// the same thing in either place. In order:
///
///   1. a registry name    -> that entry's source (the "bookmark" case)
///   2. already a URL      -> untouched
///   3. an existing file   -> untouched
///   4. host-shaped        -> `https://` in front of it
///   5. anything else      -> untouched, and `resolve.rs` will say why
///
/// Step 4 is the only guess, and it is deliberately the *last* one: a name and
/// a real file both beat it, so guessing can never shadow something that
/// actually exists.
pub fn resolve(input: &str, apps: &[AppEntry]) -> (String, String) {
    let cleaned = normalize(input);

    if let Some(e) = apps.iter().find(|e| e.name == cleaned) {
        return (e.src.clone(), e.name.clone());
    }
    if looks_like_host(cleaned) {
        return (format!("https://{cleaned}"), cleaned.to_string());
    }
    (cleaned.to_string(), cleaned.to_string())
}

/// Whether typed text is most likely a web address missing its scheme.
///
/// Nobody types `https://` any more, so `example.com` has to work; but
/// `app.wasm` is a filename that also contains a dot, and guessing wrong there
/// would turn a missing-file message into a DNS failure. The rule looks only at
/// the authority — the part before the first `/`, `?` or `#` — so a path may
/// contain whatever it likes.
fn looks_like_host(s: &str) -> bool {
    if s.is_empty()
        || s.contains("://")
        || s.starts_with('/')
        || s.starts_with('.')
        || s.contains('\\')
        || Path::new(s).exists()
    {
        return false;
    }

    let authority = s.split(['/', '?', '#']).next().unwrap_or(s);

    // `app.wasm`, `page.html`: a filename, not a host, whatever the dot says.
    let lower = authority.to_ascii_lowercase();
    if [".wasm", ".html", ".htm", ".json"]
        .iter()
        .any(|ext| lower.ends_with(ext))
    {
        return false;
    }

    let host = authority.split(':').next().unwrap_or(authority);
    host == "localhost" || (host.contains('.') && !host.ends_with('.'))
}
