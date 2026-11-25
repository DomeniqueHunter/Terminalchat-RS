//! `config.json`, compatible with the Python version. Unknown keys are kept when saving.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::ser::PrettyFormatter;
use serde_json::{Map, Value};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub account: String,
    pub password: String,
    pub character: String,
    pub remember_password: bool,
    /// Where the password is kept: flat, onetime, masterpass or systemstore (see secrets.rs).
    /// Empty in old files: flat if a password is remembered, else chosen at the next login.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub password_storage: String,
    /// The password encrypted with the master password (masterpass).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_secret: Option<String>,
    pub protocol: String,
    pub server: String,
    pub endpoint: String,
    pub default_channels: Vec<String>,
    pub show_sidebar: bool,
    pub show_ads: bool,
    /// Terminal bell for every incoming message, except in chats muted with /mute.
    pub ping: bool,
    pub log_keep_days: u32,
    pub eicons: String,
    pub eicon_height: u32,
    pub eicon_animate: bool,
    pub eicon_max_age_hours: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,

    #[serde(flatten)]
    pub extra: Map<String, Value>,

    #[serde(skip)]
    pub path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            account: String::new(),
            password: String::new(),
            character: String::new(),
            remember_password: true,
            password_storage: String::new(),
            password_secret: None,
            protocol: "wss://".into(),
            server: "chat.f-list.net".into(),
            endpoint: "/chat2".into(),
            default_channels: Vec::new(),
            show_sidebar: false,
            show_ads: true,
            ping: false,
            log_keep_days: 0,
            eicons: "inline".into(),
            eicon_height: 4,
            eicon_animate: true,
            eicon_max_age_hours: 24.0,
            cell_size: None,
            colors: None,
            data_dir: None,
            extra: Map::new(),
            path: PathBuf::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("invalid JSON in {path}: {source}")]
    Parse { path: PathBuf, source: serde_json::Error },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
}

impl Config {
    /// Loads the file; a missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let mut config = match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|source| ConfigError::Parse { path: path.to_path_buf(), source })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Config::default(),
            Err(source) => return Err(ConfigError::Read { path: path.to_path_buf(), source }),
        };
        config.path = path.to_path_buf();
        Ok(config)
    }

    /// The storage method in effect; empty when none was chosen yet.
    pub fn storage(&self) -> &str {
        if crate::secrets::METHODS.contains(&self.password_storage.as_str()) {
            &self.password_storage
        } else if self.remember_password && !self.password.is_empty() {
            crate::secrets::FLAT
        } else {
            ""
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.account.is_empty() && !self.password.is_empty() && !self.character.is_empty()
    }

    pub fn data_dir(&self) -> PathBuf {
        paths::data_dir(self.data_dir.as_deref())
    }

    pub fn websocket_url(&self) -> String {
        format!("{}{}{}", self.protocol, self.server, self.endpoint)
    }

    /// Writes the file with 4 space indentation and permissions 600, via a temporary file.
    pub fn save(&self) -> Result<(), ConfigError> {
        let write_error = |source| ConfigError::Write { path: self.path.clone(), source };
        let mut data = self.clone();
        // only flat keeps the password in the file; remember_password stays for the Python version
        let flat = self.storage() == crate::secrets::FLAT;
        data.remember_password = flat;
        if !flat {
            data.password.clear();
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(write_error)?;
        }

        let mut buffer = Vec::new();
        let formatter = PrettyFormatter::with_indent(b"    ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
        data.serialize(&mut serializer)
            .map_err(|error| write_error(io::Error::new(io::ErrorKind::InvalidData, error)))?;

        let tmp_path = self.path.with_extension("tmp");
        fs::write(&tmp_path, buffer).map_err(write_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600)).map_err(write_error)?;
        }
        fs::rename(&tmp_path, &self.path).map_err(write_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("terminalchat-test-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn missing_file_gives_defaults() {
        let config = Config::load(Path::new("/nonexistent/config.json")).unwrap();
        assert_eq!(config.server, "chat.f-list.net");
        assert!(!config.is_complete());
        assert_eq!(config.websocket_url(), "wss://chat.f-list.net/chat2");
    }

    #[test]
    fn reads_python_config_and_keeps_unknown_keys() {
        let path = temp_file("compat");
        fs::write(
            &path,
            r#"{"account": "a", "password": "p", "character": "c", "port": 8722, "default_channels": ["Development"]}"#,
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert!(config.is_complete());
        assert_eq!(config.default_channels, vec!["Development"]);

        config.save().unwrap();
        let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["port"], 8722);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn password_is_dropped_when_not_remembered() {
        let path = temp_file("nopass");
        let mut config = Config::load(&path).unwrap();
        config.password = "secret".into();
        config.remember_password = false;
        config.save().unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("secret"));
    }

    #[test]
    fn only_flat_writes_the_password_into_the_file() {
        let path = temp_file("storage");
        let mut config = Config::load(&path).unwrap();
        config.password = "secret".into();
        for (method, written) in [("flat", true), ("onetime", false), ("masterpass", false), ("systemstore", false)] {
            config.password_storage = method.into();
            config.save().unwrap();
            let text = fs::read_to_string(&path).unwrap();
            assert_eq!(text.contains("secret"), written, "{method}");
            assert_eq!(Config::load(&path).unwrap().storage(), method);
        }
        let mut old = Config::load(&path).unwrap();
        old.password_storage.clear();
        old.password = "secret".into();
        old.remember_password = true;
        assert_eq!(old.storage(), "flat", "old files with a remembered password stay flat");
    }
}
