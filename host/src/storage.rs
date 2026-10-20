//! An app's own strings — `storage-*` in the WIT, ADR-2
//! (`docs/adr/0002-per-app-storage.md`).
//!
//! Kept on disk under the app's identity when it has one to keep them under —
//! signed, and asking for `storage` — and otherwise only for as long as the app
//! is running, like everything else it holds. The file belongs to the host;
//! the guest sees keys and values, never a path.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The most one app may keep, keys and values together: 1 MiB.
pub const STORAGE_QUOTA: usize = 1 << 20;

/// How often changes are written out, at most. A guest that writes every
/// frame would otherwise write the disk sixty times a second.
const FLUSH_EVERY: Duration = Duration::from_millis(250);

pub struct Storage {
    map: BTreeMap<String, String>,
    bytes: usize,
    /// `None` for an app with no identity: nothing outlives it.
    file: Option<PathBuf>,
    dirty: bool,
    flushed: Instant,
}

impl Storage {
    /// The storage of the app known as `identity` (`<publisher>/<name>`), or a
    /// fresh, memory-only one when there is none. A file that cannot be read
    /// starts empty rather than refusing to start the app.
    pub fn open(identity: Option<&str>) -> Self {
        let file = identity.and_then(|id| {
            crate::paths::data_dir().map(|d| d.join("apps").join(id).join("storage.json"))
        });
        let map: BTreeMap<String, String> = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let bytes = map.iter().map(|(k, v)| k.len() + v.len()).sum();
        Storage {
            map,
            bytes,
            file,
            dirty: false,
            flushed: Instant::now(),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        self.map.keys().cloned().collect()
    }

    pub fn set(&mut self, key: String, value: String) -> Result<(), String> {
        let replaced = self.map.get(&key).map_or(0, |v| key.len() + v.len());
        let total = self.bytes - replaced + key.len() + value.len();
        if total > STORAGE_QUOTA {
            return Err(format!(
                "storage is full: an app may keep {} KiB",
                STORAGE_QUOTA >> 10
            ));
        }
        self.bytes = total;
        self.map.insert(key, value);
        self.dirty = true;
        Ok(())
    }

    pub fn remove(&mut self, key: &str) {
        if let Some(v) = self.map.remove(key) {
            self.bytes -= key.len() + v.len();
            self.dirty = true;
        }
    }

    /// After a frame: write the changes out if it is time, and otherwise say
    /// in how many milliseconds it will be — a frame has to come then, or a
    /// change made just before the app went idle would wait for the next tap.
    pub fn tick(&mut self) -> Option<u32> {
        if !self.dirty {
            return None;
        }
        let since = self.flushed.elapsed();
        if since >= FLUSH_EVERY {
            self.flush();
            None
        } else {
            Some((FLUSH_EVERY - since).as_millis() as u32)
        }
    }

    fn flush(&mut self) {
        self.dirty = false;
        self.flushed = Instant::now();
        let Some(file) = &self.file else {
            return;
        };
        let Ok(json) = serde_json::to_vec(&self.map) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written aside and renamed: a crash mid-write leaves the old file,
        // never half of a new one.
        let tmp = file.with_extension("json.part");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }
}

impl Drop for Storage {
    /// Leaving an app is when everything it was still holding is written.
    fn drop(&mut self) {
        if self.dirty {
            self.flush();
        }
    }
}
