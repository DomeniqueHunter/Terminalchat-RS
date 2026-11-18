//! Chats and their daily log files. The format is identical to the Python version:
//! `[YYYY-MM-DD HH:MM:SS] Sender: message`, continuation lines start with a tab.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{Duration, Local};
use ratatui::style::Style;

use crate::bbcode::{self, Seg, NameStyle, QUOTE_BAR};
use crate::sanitize::{clean_text, html_unescape, safe_path_part};
use crate::style::sender_style;

pub const USER: &str = "user";
pub const CHANNEL: &str = "channel";
pub const CONSOLE: &str = "console";
pub const AD_PREFIX: &str = "[AD] ";
pub const EMOTE_PREFIX: &str = "* ";
const CONSOLE_DIR: &str = "_console";
const CONTINUATION: char = '\t';

pub fn make_id(kind: &str, key: &str) -> String {
    format!("{kind}:{key}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chat {
    pub key: String,
    pub kind: &'static str,
    pub title: String,
    pub unread: bool,
}

impl Chat {
    pub fn id(&self) -> String {
        make_id(self.kind, &self.key)
    }
}

pub struct ChatStore {
    pub base: PathBuf,
    pub chats: Vec<Chat>,
}

impl ChatStore {
    pub fn new(character: &str, base_dir: &Path) -> ChatStore {
        ChatStore { base: base_dir.join(character), chats: Vec::new() }
    }

    pub fn get(&self, chat_id: &str) -> Option<&Chat> {
        self.chats.iter().find(|chat| chat.id() == chat_id)
    }

    pub fn get_mut(&mut self, chat_id: &str) -> Option<&mut Chat> {
        self.chats.iter_mut().find(|chat| chat.id() == chat_id)
    }

    pub fn find(&self, kind: &str, key: &str) -> Option<&Chat> {
        self.get(&make_id(kind, key))
    }

    pub fn find_user(&self, name: &str) -> Option<&Chat> {
        let name = name.trim().to_lowercase();
        self.chats.iter().find(|chat| chat.kind == USER && chat.key.to_lowercase() == name)
    }

    pub fn get_or_create(&mut self, key: &str, kind: &'static str, title: Option<&str>) -> &Chat {
        let id = make_id(kind, key);
        match self.chats.iter().position(|chat| chat.id() == id) {
            Some(index) => {
                if let Some(title) = title.filter(|title| !title.is_empty()) {
                    self.chats[index].title = title.to_string();
                }
                &self.chats[index]
            }
            None => {
                let title = title.filter(|title| !title.is_empty()).unwrap_or(key).to_string();
                self.chats.push(Chat { key: key.to_string(), kind, title, unread: false });
                self.chats.last().unwrap()
            }
        }
    }

    pub fn remove(&mut self, chat_id: &str) {
        self.chats.retain(|chat| chat.id() != chat_id);
    }

    pub fn log_dir(&self, chat: &Chat) -> PathBuf {
        match chat.kind {
            CONSOLE => self.base.join(CONSOLE_DIR),
            CHANNEL => self.base.join("channels").join(safe_path_part(&chat.key)),
            _ => self.base.join(safe_path_part(&chat.key)),
        }
    }

    /// Days that have a log file, oldest first.
    pub fn days(&self, chat: &Chat) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.log_dir(chat)) else { return Vec::new() };
        let mut days: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str().map(String::from))
            .filter_map(|name| name.strip_suffix(".log").map(String::from))
            .filter(|day| is_day(day))
            .collect();
        days.sort();
        days
    }

    /// Appends a message to the log and returns the formatted line.
    pub fn append(&self, chat: &Chat, sender: &str, message: &str) -> String {
        let line = format_line(&clean_text(sender), &clean_text(message));
        let directory = self.log_dir(chat);
        let _ = fs::create_dir_all(&directory);
        let path = directory.join(format!("{}.log", &line[1..11]));
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(encode_entry(&line).as_bytes());
        }
        line
    }

    pub fn entries(&self, chat: &Chat, day: &str) -> Vec<String> {
        read_entries(&self.log_dir(chat).join(format!("{day}.log")))
    }

    /// Deletes day files older than `keep_days` (today included). Returns how many were removed.
    pub fn cleanup(&self, keep_days: u32) -> usize {
        if keep_days == 0 || !self.base.is_dir() {
            return 0;
        }
        let oldest = (Local::now() - Duration::days(i64::from(keep_days) - 1)).format("%Y-%m-%d").to_string();
        let mut removed = 0;
        let mut stack = vec![self.base.clone()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = fs::read_dir(&directory) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let is_old = name.strip_suffix(".log").is_some_and(|day| is_day(day) && day < oldest.as_str());
                if is_old && fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }
}

fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, byte)| if index == 4 || index == 7 { *byte == b'-' } else { byte.is_ascii_digit() })
}

fn starts_entry(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 22
        && bytes[0] == b'['
        && is_day(&line[1..11])
        && bytes[11] == b' '
        && bytes[12..20].iter().enumerate().all(|(index, byte)| if index == 2 || index == 5 { *byte == b':' } else { byte.is_ascii_digit() })
        && &line[20..22] == "] "
}

pub fn encode_entry(entry: &str) -> String {
    format!("{}\n", entry.replace('\n', &format!("\n{CONTINUATION}")))
}

pub fn read_entries(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let mut entries: Vec<String> = Vec::new();
    if text.is_empty() {
        return entries;
    }
    for raw in text.split('\n') {
        let raw = raw.trim_end_matches('\r');
        let line = match raw.strip_prefix(CONTINUATION) {
            Some(rest) => format!("{CONTINUATION}{}", clean_text(rest)),
            None => clean_text(raw),
        };
        if !entries.is_empty() && !starts_entry(&line) {
            let last = entries.last_mut().unwrap();
            last.push('\n');
            last.push_str(line.strip_prefix(CONTINUATION).unwrap_or(&line));
        } else {
            entries.push(line);
        }
    }
    entries
}

pub fn format_line(sender: &str, message: &str) -> String {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    let message = html_unescape(message).replace("\r\n", "\n").replace('\r', "\n");
    let message = message.trim();
    if let Some(rest) = message.strip_prefix("/me ") {
        return format!("[{timestamp}] {EMOTE_PREFIX}{sender}: {rest}");
    }
    if let Some(rest) = message.strip_prefix("/me's ") {
        return format!("[{timestamp}] {EMOTE_PREFIX}{sender}: 's {rest}");
    }
    format!("[{timestamp}] {sender}: {message}")
}

/// Splits a log line into (timestamp, sender, message).
pub fn parse_line(line: &str) -> Option<(&str, &str, &str)> {
    if !line.starts_with('[') {
        return None;
    }
    let close = line.find(']')?;
    let timestamp = &line[..=close];
    let rest = line[close + 1..].strip_prefix(' ')?;
    let separator = rest.find(": ")?;
    let sender = &rest[..separator];
    if sender.is_empty() || sender.contains('\n') {
        return None;
    }
    Some((timestamp, sender, &rest[separator + 2..]))
}

pub fn is_ad_line(line: &str) -> bool {
    parse_line(line).is_some_and(|(_, sender, _)| sender.starts_with(AD_PREFIX))
}

/// Header (timestamp, sender) and message of a log line as segments.
pub fn split_line(line: &str, name_style: NameStyle, reveal: bool) -> (Vec<Seg>, Vec<Seg>) {
    let Some((timestamp, sender, message)) = parse_line(line) else {
        return (Vec::new(), bbcode::to_segs(line, name_style, reveal));
    };
    let base = sender_style();
    let (prefix, name, separator) = if let Some(name) = sender.strip_prefix(EMOTE_PREFIX) {
        (EMOTE_PREFIX, name, if message.starts_with("'s ") { "" } else { " " })
    } else if let Some(name) = sender.strip_prefix(AD_PREFIX) {
        (AD_PREFIX, name, ": ")
    } else {
        ("", sender, ": ")
    };
    let mut header = vec![Seg::new(format!("{timestamp} "), Style::default())];
    if !prefix.is_empty() {
        header.push(Seg::new(prefix, base));
    }
    header.push(Seg::new(name, base.patch(name_style(name))));
    header.push(Seg::new(separator, Style::default()));
    (header, bbcode::to_segs(message, name_style, reveal))
}

/// Renders a whole log line. A message that starts with a quote goes below the header.
pub fn render_line(line: &str, name_style: NameStyle, reveal: bool) -> Vec<Seg> {
    let (mut header, message) = split_line(line, name_style, reveal);
    let starts_with_quote = message.first().is_some_and(|seg| seg.text.starts_with(QUOTE_BAR));
    if !header.is_empty() && starts_with_quote {
        header.push(Seg::new("\n", Style::default()));
    }
    header.extend(message);
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bbcode::{no_names, plain_text};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn formats_normal_and_emote_lines() {
        let normal = format_line("Max", "Hallo &amp; so");
        assert!(normal.ends_with("] Max: Hallo & so"));
        assert!(format_line("Max", "/me winkt").ends_with("] * Max: winkt"));
        assert!(format_line("Max", "/me's Hut").ends_with("] * Max: 's Hut"));
    }

    #[test]
    fn parses_lines() {
        let (timestamp, sender, message) = parse_line("[2026-11-18 10:00:00] Max: hi: there").unwrap();
        assert_eq!(timestamp, "[2026-11-18 10:00:00]");
        assert_eq!(sender, "Max");
        assert_eq!(message, "hi: there");
    }

    #[test]
    fn multi_line_entries_round_trip() {
        let dir = temp_dir("roundtrip");
        let mut store = ChatStore::new("Me", &dir);
        let chat = store.get_or_create("Bob", USER, None).clone();
        store.append(&chat, "Bob", "eins\nzwei");
        store.append(&chat, "Me", "drei");
        let day = Local::now().format("%Y-%m-%d").to_string();
        let entries = store.entries(&chat, &day);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].ends_with("Bob: eins\nzwei"));
        assert_eq!(store.days(&chat), vec![day]);
    }

    #[test]
    fn renders_header_and_message() {
        let text = plain_text(&render_line("[2026-11-18 10:00:00] * Max: winkt [b]fett[/b]", &no_names, false));
        assert_eq!(text, "[2026-11-18 10:00:00] * Max winkt fett");
    }

    #[test]
    fn cleanup_removes_old_days_only() {
        let dir = temp_dir("cleanup");
        let mut store = ChatStore::new("Me", &dir);
        let chat = store.get_or_create("Bob", USER, None).clone();
        let directory = store.log_dir(&chat);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("2000-01-01.log"), "x").unwrap();
        store.append(&chat, "Bob", "neu");
        assert_eq!(store.cleanup(30), 1);
        assert_eq!(store.days(&chat).len(), 1);
    }
}
