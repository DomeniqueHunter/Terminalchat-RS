//! Small JSON files next to the logs, compatible with the Python version:
//! pins, own status, ad visibility, ad drafts and known genders. `muted.json` (chats without
//! ping) is new in this version.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const VALID_STATUSES: [&str; 6] = ["online", "looking", "busy", "dnd", "idle", "away"];
pub const DEFAULT_STATUS: &str = "online";
pub const MAX_STATUS_MESSAGE: usize = 255;

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write_atomic(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub kind: String,
    pub key: String,
    pub title: Option<String>,
}

pub struct Pins {
    path: PathBuf,
    pub items: Vec<Pin>,
}

impl Pins {
    pub fn load(path: PathBuf) -> Pins {
        let items = match read_json(&path) {
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(|item| {
                    Some(Pin {
                        kind: item.get("kind")?.as_str()?.to_string(),
                        key: item.get("key")?.as_str()?.to_string(),
                        title: item.get("title").and_then(Value::as_str).map(String::from),
                    })
                })
                .collect(),
            _ => Vec::new(),
        };
        Pins { path, items }
    }

    fn save(&self) {
        let list: Vec<Value> = self
            .items
            .iter()
            .map(|pin| json!({"kind": pin.kind, "key": pin.key, "title": pin.title}))
            .collect();
        let text = serde_json::to_string_pretty(&list).unwrap_or_default();
        write_atomic(&self.path, &text);
    }

    pub fn is_pinned(&self, kind: &str, key: &str) -> bool {
        self.items.iter().any(|pin| pin.kind == kind && pin.key == key)
    }

    pub fn pin(&mut self, kind: &str, key: &str, title: &str) -> bool {
        if self.is_pinned(kind, key) {
            return false;
        }
        self.items.push(Pin { kind: kind.into(), key: key.into(), title: Some(title.into()) });
        self.save();
        true
    }

    pub fn unpin(&mut self, kind: &str, key: &str) -> bool {
        if !self.is_pinned(kind, key) {
            return false;
        }
        self.items.retain(|pin| !(pin.kind == kind && pin.key == key));
        self.save();
        true
    }
}

pub struct OwnStatus {
    path: PathBuf,
    pub status: String,
    pub message: String,
}

impl OwnStatus {
    pub fn load(path: PathBuf) -> OwnStatus {
        let mut own = OwnStatus { path, status: DEFAULT_STATUS.into(), message: String::new() };
        if let Some(data) = read_json(&own.path) {
            let status = data.get("status").and_then(Value::as_str).unwrap_or(DEFAULT_STATUS).to_lowercase();
            if VALID_STATUSES.contains(&status.as_str()) {
                own.status = status;
            }
            let message = data.get("message").and_then(Value::as_str).unwrap_or("");
            own.message = message.chars().take(MAX_STATUS_MESSAGE).collect();
        }
        own
    }

    pub fn set(&mut self, status: &str, message: &str) {
        self.status = if VALID_STATUSES.contains(&status) { status.into() } else { DEFAULT_STATUS.into() };
        self.message = message.trim().chars().take(MAX_STATUS_MESSAGE).collect();
        let text = serde_json::to_string_pretty(&json!({"status": self.status, "message": self.message})).unwrap_or_default();
        write_atomic(&self.path, &text);
    }

    pub fn is_default(&self) -> bool {
        self.status == DEFAULT_STATUS && self.message.is_empty()
    }
}

pub struct AdSettings {
    path: PathBuf,
    default: bool,
    channels: BTreeMap<String, bool>,
}

impl AdSettings {
    /// Ads shown in channels without an own choice.
    pub fn set_default(&mut self, default: bool) {
        self.default = default;
    }

    pub fn load(path: PathBuf, default: bool) -> AdSettings {
        let channels = match read_json(&path) {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(key, value)| (key.to_lowercase(), value.as_bool().unwrap_or(false)))
                .collect(),
            _ => BTreeMap::new(),
        };
        AdSettings { path, default, channels }
    }

    pub fn visible(&self, code: &str) -> bool {
        self.channels.get(&code.to_lowercase()).copied().unwrap_or(self.default)
    }

    pub fn set_visible(&mut self, code: &str, visible: bool) {
        self.channels.insert(code.to_lowercase(), visible);
        write_atomic(&self.path, &serde_json::to_string_pretty(&self.channels).unwrap_or_default());
    }
}

/// Chats that do not ping (`/mute`), by chat id.
pub struct Muted {
    path: PathBuf,
    chats: BTreeSet<String>,
}

impl Muted {
    pub fn load(path: PathBuf) -> Muted {
        let chats = match read_json(&path) {
            Some(Value::Array(ids)) => ids.iter().filter_map(Value::as_str).map(String::from).collect(),
            _ => BTreeSet::new(),
        };
        Muted { path, chats }
    }

    pub fn contains(&self, chat_id: &str) -> bool {
        self.chats.contains(chat_id)
    }

    pub fn set(&mut self, chat_ids: &[String], muted: bool) {
        for id in chat_ids {
            if muted {
                self.chats.insert(id.clone());
            } else {
                self.chats.remove(id);
            }
        }
        write_atomic(&self.path, &serde_json::to_string_pretty(&self.chats).unwrap_or_default());
    }
}

pub struct AdDrafts {
    path: PathBuf,
    drafts: BTreeMap<String, String>,
    dirty: bool,
}

impl AdDrafts {
    pub fn load(path: PathBuf) -> AdDrafts {
        let drafts = match read_json(&path) {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(key, value)| (key.to_lowercase(), value.as_str().unwrap_or("").to_string()))
                .collect(),
            _ => BTreeMap::new(),
        };
        AdDrafts { path, drafts, dirty: false }
    }

    pub fn get(&self, code: &str) -> String {
        self.drafts.get(&code.to_lowercase()).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, code: &str, text: &str) {
        let key = code.to_lowercase();
        if self.drafts.get(&key).map(String::as_str).unwrap_or("") == text {
            return;
        }
        if text.is_empty() {
            self.drafts.remove(&key);
        } else {
            self.drafts.insert(key, text.to_string());
        }
        self.dirty = true;
    }

    pub fn save(&mut self) {
        if self.dirty {
            write_atomic(&self.path, &serde_json::to_string_pretty(&self.drafts).unwrap_or_default());
            self.dirty = false;
        }
    }
}

pub struct GenderStore {
    path: PathBuf,
    data: BTreeMap<String, String>,
    dirty: bool,
}

impl GenderStore {
    pub fn load(path: PathBuf) -> GenderStore {
        let data = match read_json(&path) {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(key, value)| (key.clone(), value.as_str().unwrap_or("").to_string()))
                .collect(),
            _ => BTreeMap::new(),
        };
        GenderStore { path, data, dirty: false }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.data.get(&name.trim().to_lowercase()).map(String::as_str)
    }

    pub fn set(&mut self, name: &str, gender: Option<&str>) {
        let Some(gender) = gender.filter(|gender| !gender.is_empty()) else { return };
        if name.is_empty() {
            return;
        }
        let key = name.trim().to_lowercase();
        if self.data.get(&key).map(String::as_str) != Some(gender) {
            self.data.insert(key, gender.to_string());
            self.dirty = true;
        }
    }

    pub fn save(&mut self) {
        if self.dirty {
            write_atomic(&self.path, &serde_json::to_string(&self.data).unwrap_or_default());
            self.dirty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-stores-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("file.json")
    }

    #[test]
    fn pins_persist() {
        let path = temp_file("pins");
        let mut pins = Pins::load(path.clone());
        assert!(pins.pin("channel", "Development", "Development"));
        assert!(!pins.pin("channel", "Development", "Development"));
        assert_eq!(Pins::load(path).items.len(), 1);
    }

    #[test]
    fn own_status_clamps_and_persists() {
        let path = temp_file("status");
        let mut own = OwnStatus::load(path.clone());
        assert!(own.is_default());
        own.set("busy", "  zurück gleich ");
        let again = OwnStatus::load(path);
        assert_eq!((again.status.as_str(), again.message.as_str()), ("busy", "zurück gleich"));
    }

    #[test]
    fn ads_default_and_override() {
        let path = temp_file("ads");
        let mut ads = AdSettings::load(path, true);
        assert!(ads.visible("Dev"));
        ads.set_visible("Dev", false);
        assert!(!ads.visible("dev"));
    }

    #[test]
    fn genders_are_case_insensitive() {
        let path = temp_file("genders");
        let mut genders = GenderStore::load(path.clone());
        genders.set("Alice", Some("Female"));
        genders.save();
        assert_eq!(GenderStore::load(path).get("ALICE"), Some("Female"));
    }

    #[test]
    fn muted_chats_survive_a_restart() {
        let path = temp_file("muted");
        let mut muted = Muted::load(path.clone());
        muted.set(&["channel:Frontpage".into(), "user:Bo".into()], true);
        muted.set(&["user:Bo".into()], false);
        let loaded = Muted::load(path);
        assert!(loaded.contains("channel:Frontpage") && !loaded.contains("user:Bo"));
    }
}
