//! File locations, identical to the Python version (XDG base directories).

use std::env;
use std::path::PathBuf;

pub const APP_NAME: &str = "terminalchat";

fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

fn non_empty_var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    match non_empty_var(variable) {
        Some(value) => PathBuf::from(value),
        None => home().join(fallback),
    }
}

/// `TERMINALCHAT_CONFIG`, otherwise `$XDG_CONFIG_HOME/terminalchat/config.json`.
pub fn config_file() -> PathBuf {
    match non_empty_var("TERMINALCHAT_CONFIG") {
        Some(value) => expand_home(&value),
        None => xdg_dir("XDG_CONFIG_HOME", ".config").join(APP_NAME).join("config.json"),
    }
}

/// `TERMINALCHAT_DATA`, then `data_dir` from the config, otherwise `$XDG_DATA_HOME/terminalchat`.
pub fn data_dir(configured: Option<&str>) -> PathBuf {
    let override_dir = non_empty_var("TERMINALCHAT_DATA").or_else(|| configured.filter(|v| !v.is_empty()).map(String::from));
    match override_dir {
        Some(value) => expand_home(&value),
        None => xdg_dir("XDG_DATA_HOME", ".local/share").join(APP_NAME),
    }
}

/// `$XDG_CACHE_HOME/terminalchat`, used for eicons, avatars and thumbnails.
pub fn cache_dir() -> PathBuf {
    xdg_dir("XDG_CACHE_HOME", ".cache").join(APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_home_prefix() {
        let expected = home().join("x/y");
        assert_eq!(expand_home("~/x/y"), expected);
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn configured_data_dir_is_used_without_override() {
        if env::var_os("TERMINALCHAT_DATA").is_none() {
            assert_eq!(data_dir(Some("/tmp/tc-data")), PathBuf::from("/tmp/tc-data"));
        }
    }
}
