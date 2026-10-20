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
    /// The component's SHA-256, hex. When present, bytes that do not match are
    /// refused before they are compiled, and a remote component is cached
    /// under it for opening offline (`resolve::classify_pinned`).
    #[serde(default)]
    pub sha256: Option<String>,
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
    // A registry decides what gets run, so it travels over TLS like a
    // component does.
    crate::resolve::require_tls(url).ok()?;
    // `into_string` caps the body at 10 MB itself; the agent bounds the wait.
    crate::resolve::http()
        .get(url)
        .call()
        .ok()?
        .into_string()
        .ok()
}

/// The registry when none is named on the command line, where it came from,
/// and — for the hosted one — the URL to refresh it from. In order:
///
///   1. `NIMADIR_REGISTRY`, a path or URL — set once instead of passed each time
///   2. `apps.json` / `apps.list` in the working directory — a checkout
///   3. `apps.json` in the user's config directory — a shell configured by hand
///   4. the hosted registry (`hosted_url`): its saved copy now, a fresh one
///      fetched in the background — or the built-in list until that arrives
///   5. the built-in list
///
/// The working directory used to be the only place looked, so a shell started
/// from anywhere but the checkout had no registry and nothing to open.
pub fn load_default() -> (Vec<AppEntry>, String, Option<String>) {
    if let Some(src) = std::env::var("NIMADIR_REGISTRY")
        .ok()
        .filter(|s| !s.is_empty())
    {
        let (apps, label) = load_from(&src);
        return (apps, label, None);
    }
    if Path::new(MANIFEST_JSON).is_file() || Path::new(MANIFEST).is_file() {
        return (load(Path::new(".")), describe_default(), None);
    }
    if let Some(user) = crate::paths::config_dir().map(|d| d.join(MANIFEST_JSON)) {
        if user.is_file() {
            let (apps, label) = load_from(&user.to_string_lossy());
            return (apps, label, None);
        }
    }
    if let Some(url) = hosted_url() {
        let saved = hosted_copy(&url)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| parse_json(&t))
            .filter(|apps| !apps.is_empty());
        return match saved {
            Some(apps) => (apps, format!("{url} (saved copy)"), Some(url)),
            None => (
                builtin(),
                format!("built-in, until {url} arrives"),
                Some(url),
            ),
        };
    }
    (builtin(), "built-in".to_string(), None)
}

/// The project's own registry, published by `.github/workflows/registry.yml`,
/// for a shell nothing else configures. `NIMADIR_HOSTED_REGISTRY` replaces it
/// — a fork's, or the test suite's — and an empty value turns it off.
const HOSTED_REGISTRY: &str = "https://nodirbek-abdulaxadov.github.io/nimadir/apps.json";

pub fn hosted_url() -> Option<String> {
    match std::env::var("NIMADIR_HOSTED_REGISTRY") {
        Ok(url) => (!url.is_empty()).then_some(url),
        Err(_) => Some(HOSTED_REGISTRY.to_string()),
    }
}

/// Where the last good copy of a hosted registry is kept, one file per URL.
fn hosted_copy(url: &str) -> Option<std::path::PathBuf> {
    let key = &crate::resolve::sha256_hex(url.as_bytes())[..16];
    crate::paths::cache_dir().map(|d| d.join("registry").join(format!("{key}.json")))
}

/// Fetch a hosted registry and keep a copy for the next start. `None` when it
/// could not be reached or held nothing — what the shell has stays.
pub fn fetch_hosted(url: &str) -> Option<Vec<AppEntry>> {
    let text = fetch(url)?;
    let apps = parse_json(&text);
    if apps.is_empty() {
        return None;
    }
    if let Some(path) = hosted_copy(url) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, text);
    }
    Some(apps)
}

/// Make sure there is a start page to go home to: a registry that lists no
/// `store` gets the one built into the binary, when there is one. Without it
/// the shell would open on its fallback list — host UI, not the start page.
pub fn ensure_start_page(apps: &mut Vec<AppEntry>) {
    if crate::resolve::has_builtin_store() && !apps.iter().any(|e| e.name == STORE) {
        apps.insert(0, builtin_store());
    }
}

const STORE: &str = crate::shell::STORE_APP;

fn builtin_store() -> AppEntry {
    AppEntry {
        name: STORE.into(),
        src: format!("{}{STORE}", crate::resolve::BUILTIN_SCHEME),
        description: "The start page, built into the shell.".into(),
        // Inside the binary: nothing crossed a network to tamper with.
        sha256: None,
    }
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

/// Parse the manifest: one entry per line, `name | src | description | sha256`.
/// Blank lines and `#` comments are skipped; description and pin are optional.
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
        let sha256 = parts.next().filter(|h| !h.is_empty()).map(str::to_string);
        if name.is_empty() || src.is_empty() {
            continue;
        }
        out.push(AppEntry {
            name,
            src,
            description,
            sha256,
        });
    }
    out
}

/// Entries used when no registry was found or usable: only what works from
/// anywhere, which is what the binary carries. It used to list the sample apps
/// by their paths in a checkout — links that were dead everywhere else, which
/// is exactly where this list is needed.
pub fn builtin() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    ensure_start_page(&mut apps);
    apps
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
