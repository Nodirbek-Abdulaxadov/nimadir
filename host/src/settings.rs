//! What the shell keeps on disk, for the settings page and for `--kept` /
//! `--clear` / `--clear-cache`: each app's storage (ADR-2), and the caches.
//!
//! No UI here: the window draws it (`gui.rs`), the CLI prints it, and both go
//! through these functions.

use std::path::Path;

/// One app's kept data.
pub struct Kept {
    /// `<publisher>/<name>`.
    pub identity: String,
    pub bytes: u64,
}

/// Every app that keeps something, by identity.
pub fn kept() -> Vec<Kept> {
    let Some(root) = crate::paths::data_dir().map(|d| d.join("apps")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for publisher in entries(&root) {
        for app in entries(&publisher) {
            let (Some(p), Some(a)) = (name_of(&publisher), name_of(&app)) else {
                continue;
            };
            out.push(Kept {
                identity: format!("{p}/{a}"),
                bytes: size(&app),
            });
        }
    }
    out.sort_by(|a, b| a.identity.cmp(&b.identity));
    out
}

/// Remove everything an app kept. The identity is checked before it becomes a
/// path: two parts of `a-z`, `0-9` and `-`, so nothing typed or listed can
/// reach outside the app's own directory.
pub fn clear(identity: &str) -> Result<(), String> {
    let valid = identity.split('/').count() == 2
        && identity.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
    if !valid {
        return Err(format!(
            "{identity:?} is not an app identity (<publisher>/<name>)"
        ));
    }
    let dir = crate::paths::data_dir()
        .ok_or("no data directory")?
        .join("apps")
        .join(identity);
    if !dir.is_dir() {
        return Err(format!("{identity} keeps nothing"));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not clear {identity}: {e}"))?;
    crate::diag::line(&format!("cleared what {identity} kept"));
    Ok(())
}

/// What the caches hold: downloaded components, compiled code, registry copies.
pub fn cache_bytes() -> u64 {
    crate::paths::cache_dir().map_or(0, |d| size(&d))
}

/// Empty the caches. Everything in them can be fetched or compiled again;
/// the next opens are slower, nothing else.
pub fn clear_cache() -> Result<(), String> {
    let Some(dir) = crate::paths::cache_dir() else {
        return Ok(());
    };
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("could not clear the cache: {e}"))?;
    }
    crate::diag::line("cleared the cache");
    Ok(())
}

/// `1.2 MiB`, `340 KiB`, `12 bytes`.
pub fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MiB", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{} KiB", b >> 10),
        b => format!("{b} bytes"),
    }
}

fn entries(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn name_of(path: &Path) -> Option<&str> {
    path.file_name().and_then(|n| n.to_str())
}

fn size(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}
