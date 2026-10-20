//! Whether a newer nimadir has been released (ROADMAP S8-2).
//!
//! Asked only when the user asks — the settings page's button, or
//! `--check-update` — never on its own: a shell that phones home at start is
//! telling a server when it is used.

use std::io::Read;

use serde::Deserialize;

/// Where releases are announced. `NIMADIR_UPDATE_URL` replaces it — a fork's,
/// or the test suite's.
const RELEASES: &str = "https://api.github.com/repos/Nodirbek-Abdulaxadov/nimadir/releases/latest";

/// What the check found.
pub enum Verdict {
    /// This is the newest release, or newer than any.
    Current,
    Newer {
        version: String,
        page: String,
    },
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
}

/// Ask where releases are announced, and compare with this build. Blocks: run
/// it off the UI thread.
pub fn check() -> Result<Verdict, String> {
    let url = std::env::var("NIMADIR_UPDATE_URL").unwrap_or_else(|_| RELEASES.into());
    crate::resolve::require_tls(&url).map_err(|e| e.to_string())?;
    let response = crate::resolve::http()
        .get(&url)
        // GitHub's API refuses a request with no User-Agent.
        .set("User-Agent", concat!("nimadir/", env!("CARGO_PKG_VERSION")))
        .call();
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err("no release has been published yet".into()),
        Err(e) => return Err(format!("could not reach {url}: {e}")),
    };
    let mut body = Vec::new();
    response
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut body)
        .map_err(|e| format!("reading {url}: {e}"))?;
    let release: Release = serde_json::from_slice(&body)
        .map_err(|e| format!("{url} did not describe a release: {e}"))?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    Ok(if newer(&version, env!("CARGO_PKG_VERSION")) {
        Verdict::Newer {
            version,
            page: release.html_url,
        }
    } else {
        Verdict::Current
    })
}

/// Whether `a` is a later version than `b`, by semver's precedence:
/// `1.0.0-beta.2` > `1.0.0-beta.1`, and `1.0.0` > either.
pub fn newer(a: &str, b: &str) -> bool {
    key(a) > key(b)
}

/// A sort key for a version: the three numbers, then whether it is a release
/// (which outranks any pre-release of it), then the pre-release's parts —
/// numbers below words, and numbers compared as numbers.
fn key(v: &str) -> ([u64; 3], bool, Vec<(bool, u64, String)>) {
    let (core, pre) = v.split_once('-').unwrap_or((v, ""));
    let mut numbers = [0u64; 3];
    for (slot, part) in numbers.iter_mut().zip(core.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    let parts = pre
        .split('.')
        .filter(|p| !p.is_empty())
        .map(|p| match p.parse::<u64>() {
            Ok(n) => (false, n, String::new()),
            Err(_) => (true, 0, p.to_string()),
        })
        .collect();
    (numbers, pre.is_empty(), parts)
}
