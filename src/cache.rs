//! One small cache file for the answers that are slow to get and rarely change: shell versions,
//! GPU names, Wi-Fi network names. Lines of `key<TAB>value`, read once, written back only when
//! something changed. The write goes to a temporary file that replaces the cache, so several
//! haikufetch processes starting at once (a terminal opening many panes) never read half a file.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use crate::paths;

struct Store {
    entries: BTreeMap<String, String>,
    dirty: bool,
}

fn path() -> Option<PathBuf> {
    paths::cache_dir().map(|dir| dir.join("haikufetch/cache.tsv"))
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let entries = path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| {
                text.lines()
                    .filter_map(|line| line.split_once('\t'))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Mutex::new(Store {
            entries,
            dirty: false,
        })
    })
}

pub fn get(key: &str) -> Option<String> {
    store().lock().ok()?.entries.get(key).cloned()
}

/// Remembers a value. Tabs and newlines would break the format, so they become spaces
pub fn set(key: &str, value: &str) {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    if let Ok(mut store) = store().lock() {
        let (key, value) = (clean(key), clean(value));
        if store.entries.get(&key) != Some(&value) {
            store.entries.insert(key, value);
            store.dirty = true;
        }
    }
}

/// Drops every entry whose key starts with `prefix`
pub fn forget(prefix: &str) {
    if let Ok(mut store) = store().lock() {
        let before = store.entries.len();
        store.entries.retain(|k, _| !k.starts_with(prefix));
        store.dirty |= store.entries.len() != before;
    }
}

/// Writes the cache back if anything changed. Failures are ignored: the answer is just looked up
/// again next time
pub fn flush() {
    let Some(path) = path() else { return };
    let Ok(mut store) = store().lock() else {
        return;
    };
    if !store.dirty {
        return;
    }
    let mut text = String::new();
    for (key, value) in &store.entries {
        text.push_str(key);
        text.push('\t');
        text.push_str(value);
        text.push('\n');
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let temporary = path.with_extension(format!("tsv.{}", std::process::id()));
    if std::fs::write(&temporary, text)
        .and_then(|()| std::fs::rename(&temporary, &path))
        .is_err()
    {
        let _ = std::fs::remove_file(&temporary);
    }
    store.dirty = false;
}

/// Deletes the whole cache file, for --clear-cache
pub fn clear() {
    if let Ok(mut store) = store().lock() {
        store.entries.clear();
        store.dirty = false;
    }
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}
