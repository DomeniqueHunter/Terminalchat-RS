//! Who is online, friends, bookmarks, channel members and operators.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    pub status: String,
    pub is_op: bool,
}

#[derive(Default)]
pub struct Presence {
    online: HashMap<String, String>,
    status: HashMap<String, String>,
    messages: HashMap<String, String>,
    pub friends: HashSet<String>,
    pub bookmarks: HashSet<String>,
    members: HashMap<String, HashMap<String, String>>,
    ops: HashMap<String, HashSet<String>>,
}

impl Presence {
    pub fn set_online(&mut self, name: &str, status: &str, message: &str) {
        let key = name.to_lowercase();
        self.online.insert(key.clone(), name.to_string());
        self.status.insert(key.clone(), if status.is_empty() { "online".into() } else { status.into() });
        self.messages.insert(key, message.to_string());
    }

    pub fn set_offline(&mut self, name: &str) {
        let key = name.to_lowercase();
        self.online.remove(&key);
        self.status.remove(&key);
        self.messages.remove(&key);
        for members in self.members.values_mut() {
            members.remove(&key);
        }
    }

    pub fn set_status(&mut self, name: &str, status: &str, message: &str) {
        let key = name.to_lowercase();
        if self.online.contains_key(&key) {
            self.status.insert(key.clone(), if status.is_empty() { "online".into() } else { status.into() });
            self.messages.insert(key, message.to_string());
        }
    }

    /// Status and status message of an online character.
    pub fn status_of(&self, name: &str) -> Option<(String, String)> {
        let key = name.to_lowercase();
        if !self.online.contains_key(&key) {
            return None;
        }
        Some((
            self.status.get(&key).cloned().unwrap_or_else(|| "online".into()),
            self.messages.get(&key).cloned().unwrap_or_default(),
        ))
    }

    pub fn find_online(&self, name: &str) -> Option<String> {
        self.online.get(&name.trim().to_lowercase()).cloned()
    }

    pub fn set_bookmarks(&mut self, names: &[String]) {
        self.bookmarks = names.iter().map(|name| name.to_lowercase()).collect();
    }

    pub fn add_bookmark(&mut self, name: &str) {
        self.bookmarks.insert(name.to_lowercase());
    }

    pub fn remove_bookmark(&mut self, name: &str) {
        self.bookmarks.remove(&name.to_lowercase());
    }

    pub fn set_friends(&mut self, names: &[String]) {
        self.friends = names.iter().map(|name| name.to_lowercase()).collect();
    }

    fn ensure_online(&mut self, name: &str) {
        let key = name.to_lowercase();
        self.online.entry(key.clone()).or_insert_with(|| name.to_string());
        self.status.entry(key).or_insert_with(|| "online".into());
    }

    pub fn set_members(&mut self, code: &str, names: &[String]) {
        for name in names {
            self.ensure_online(name);
        }
        self.members.insert(code.to_string(), names.iter().map(|name| (name.to_lowercase(), name.clone())).collect());
    }

    pub fn add_member(&mut self, code: &str, name: &str) {
        self.ensure_online(name);
        self.members.entry(code.to_string()).or_default().insert(name.to_lowercase(), name.to_string());
    }

    pub fn remove_member(&mut self, code: &str, name: &str) {
        if let Some(members) = self.members.get_mut(code) {
            members.remove(&name.to_lowercase());
        }
    }

    pub fn drop_channel(&mut self, code: &str) {
        self.members.remove(code);
        self.ops.remove(code);
    }

    pub fn set_ops(&mut self, code: &str, names: &[String]) {
        self.ops.insert(code.to_string(), names.iter().filter(|n| !n.is_empty()).map(|n| n.to_lowercase()).collect());
    }

    pub fn add_op(&mut self, code: &str, name: &str) {
        self.ops.entry(code.to_string()).or_default().insert(name.to_lowercase());
    }

    pub fn remove_op(&mut self, code: &str, name: &str) {
        if let Some(ops) = self.ops.get_mut(code) {
            ops.remove(&name.to_lowercase());
        }
    }

    /// Names starting with `prefix`, members of the channel first.
    pub fn complete_names(&self, prefix: &str, code: Option<&str>) -> Vec<String> {
        let lowered = prefix.to_lowercase();
        let members = code.and_then(|code| self.members.get(code));
        let mut first: Vec<String> = members
            .map(|members| members.iter().filter(|(key, _)| key.starts_with(&lowered)).map(|(_, name)| name.clone()).collect())
            .unwrap_or_default();
        let mut rest: Vec<String> = self
            .online
            .iter()
            .filter(|(key, _)| key.starts_with(&lowered) && !members.is_some_and(|members| members.contains_key(*key)))
            .map(|(_, name)| name.clone())
            .collect();
        first.sort_by_key(|name| name.to_lowercase());
        rest.sort_by_key(|name| name.to_lowercase());
        first.extend(rest);
        first
    }

    fn entries_for(&self, keys: impl Iterator<Item = String>) -> Vec<Entry> {
        let mut entries: Vec<Entry> = keys
            .filter_map(|key| {
                let name = self.online.get(&key)?.clone();
                let status = self.status.get(&key).cloned().unwrap_or_else(|| "online".into());
                Some(Entry { name, status, is_op: false })
            })
            .collect();
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        entries
    }

    pub fn online_friends(&self) -> Vec<Entry> {
        self.entries_for(self.friends.iter().cloned())
    }

    pub fn online_bookmarks(&self) -> Vec<Entry> {
        self.entries_for(self.bookmarks.difference(&self.friends).cloned())
    }

    /// Members of a channel, operators first; `None` while the list has not arrived.
    pub fn channel_members(&self, code: &str) -> Option<Vec<Entry>> {
        let members = self.members.get(code)?;
        let ops = self.ops.get(code);
        let mut entries: Vec<Entry> = members
            .iter()
            .map(|(key, name)| Entry {
                name: name.clone(),
                status: self.status.get(key).cloned().unwrap_or_else(|| "online".into()),
                is_op: ops.is_some_and(|ops| ops.contains(key)),
            })
            .collect();
        entries.sort_by_key(|entry| (!entry.is_op, entry.name.to_lowercase()));
        Some(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friends_and_members() {
        let mut presence = Presence::default();
        presence.set_friends(&["Alice".into()]);
        presence.set_online("Alice", "looking", "");
        presence.set_members("Dev", &["Alice".into(), "Bob".into()]);
        presence.set_ops("Dev", &["Bob".into()]);
        assert_eq!(presence.online_friends()[0].status, "looking");
        let members = presence.channel_members("Dev").unwrap();
        assert_eq!(members[0].name, "Bob");
        assert!(members[0].is_op);
    }

    #[test]
    fn completes_members_first() {
        let mut presence = Presence::default();
        presence.set_online("Max Zebra", "online", "");
        presence.set_online("Maria", "online", "");
        presence.set_members("Dev", &["Max Zebra".into()]);
        assert_eq!(presence.complete_names("ma", Some("Dev")), vec!["Max Zebra", "Maria"]);
    }

    #[test]
    fn offline_removes_from_channels() {
        let mut presence = Presence::default();
        presence.set_members("Dev", &["Bob".into()]);
        presence.set_offline("Bob");
        assert!(presence.channel_members("Dev").unwrap().is_empty());
        assert!(presence.status_of("Bob").is_none());
    }
}
