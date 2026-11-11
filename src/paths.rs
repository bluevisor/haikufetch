//! Where config and cache files live. XDG everywhere (also on macOS, like most command-line
//! tools), Haiku's own folders on Haiku.

use std::path::PathBuf;

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(fallback)))
}

/// ~/.config, or $XDG_CONFIG_HOME. Haiku keeps settings in ~/config/settings
pub fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "haiku") {
        return home().map(|h| h.join("config/settings"));
    }
    xdg("XDG_CONFIG_HOME", ".config")
}

/// ~/.cache, or $XDG_CACHE_HOME. Haiku: ~/config/cache
pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(target_os = "haiku") {
        return home().map(|h| h.join("config/cache"));
    }
    xdg("XDG_CACHE_HOME", ".cache")
}

/// Where rustfetch 0.x (this project's old name) kept its config, each holding rustfetch/: the
/// same config folder, and on macOS Application Support
pub fn legacy_config_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = config_dir().into_iter().collect();
    if cfg!(target_os = "macos")
        && let Some(home) = home()
    {
        dirs.push(home.join("Library/Application Support"));
    }
    dirs
}
