//! The chat view: a window of at most 600 log lines, loaded in blocks of 300 lines.
//! Scrolling to the top loads older blocks, scrolling to the bottom of a non-live window loads
//! newer ones. Scrolling is measured in rows from the bottom.

use std::collections::HashSet;

use chrono::Local;
use ratatui::style::Style;

use crate::bbcode::{NameStyle, Seg};
use crate::images::{self, Key};
use crate::rows::wrap;
use crate::store::{is_ad_line, render_line, Chat, ChatStore, CHANNEL};
use crate::stores::AdSettings;
use crate::style::dim;

pub const BLOCK_LINES: usize = 300;
pub const MIN_OPEN_LINES: usize = 20;
pub const MAX_LOADED_LINES: usize = 600;
const RULE_WIDTH: usize = 24;
const MARKER: char = '\u{E000}';

pub type EntryKey = (String, usize);

pub struct Source<'a> {
    pub store: &'a ChatStore,
    pub ads: &'a AdSettings,
    pub chat: &'a Chat,
    pub name_style: NameStyle<'a>,
    pub image_width: &'a dyn Fn(&Key) -> u16,
}

impl Source<'_> {
    fn entries(&self, day: &str) -> Vec<String> {
        let mut lines = self.store.entries(self.chat, day);
        if self.chat.kind == CHANNEL && !self.ads.visible(&self.chat.key) {
            lines.retain(|line| !is_ad_line(line));
        }
        lines
    }

    fn days(&self) -> Vec<String> {
        self.store.days(self.chat)
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub day: String,
    pub start: usize,
    pub end: usize,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Text,
    Blank,
    Rule,
    LogStart,
    /// First row of an image strip; `Row::images` lists the images.
    Image,
    ImageCont,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub segs: Vec<Seg>,
    pub key: Option<EntryKey>,
    pub kind: RowKind,
    pub images: Vec<Key>,
    /// For images inside a text line: (column, width) of each image; empty for strips.
    pub slots: Vec<(u16, u16)>,
}

#[derive(Default)]
pub struct Pager {
    pub chat_id: Option<String>,
    pub blocks: Vec<Block>,
    pub live: bool,
    /// Rows between the bottom of the view and the bottom of the loaded lines.
    pub scroll: usize,
    pub reveal: HashSet<EntryKey>,
    pub log_start: bool,
    pub width: usize,
    pub height: usize,
    pub total_rows: usize,
    /// Rows reserved below a message for its images, 0 = images off.
    pub image_rows: usize,
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

impl Pager {
    pub fn reset(&mut self) {
        *self = Pager { width: self.width, height: self.height, live: true, ..Pager::default() };
    }

    fn loaded_lines(&self) -> usize {
        self.blocks.iter().map(|block| block.end - block.start).sum()
    }

    fn build_block(&self, src: &Source, day: &str, start: usize, end: usize, lines: Vec<String>) -> Block {
        let _ = src;
        Block { day: day.to_string(), start, end, lines }
    }

    fn older_range(&self, src: &Source) -> Option<Block> {
        let top = self.blocks.first()?;
        if top.start > 0 {
            let lines = src.entries(&top.day);
            let start = top.start.saturating_sub(BLOCK_LINES);
            let end = top.start.min(lines.len());
            return Some(Block { day: top.day.clone(), start, end, lines: lines[start.min(end)..end].to_vec() });
        }
        for day in src.days().into_iter().rev().filter(|day| *day < top.day) {
            let lines = src.entries(&day);
            if !lines.is_empty() {
                let start = lines.len().saturating_sub(BLOCK_LINES);
                return Some(Block { day, start, end: lines.len(), lines: lines[start..].to_vec() });
            }
        }
        None
    }

    fn newer_range(&self, src: &Source) -> Option<Block> {
        let bottom = self.blocks.last()?;
        let lines = src.entries(&bottom.day);
        if bottom.end < lines.len() {
            let end = lines.len().min(bottom.end + BLOCK_LINES);
            return Some(Block { day: bottom.day.clone(), start: bottom.end, end, lines: lines[bottom.end..end].to_vec() });
        }
        for day in src.days().into_iter().filter(|day| *day > bottom.day) {
            let lines = src.entries(&day);
            if !lines.is_empty() {
                let end = lines.len().min(BLOCK_LINES);
                return Some(Block { day, start: 0, end, lines: lines[..end].to_vec() });
            }
        }
        None
    }

    /// Shows a chat, starting with the given day (default: today).
    pub fn open(&mut self, src: &Source, chat_id: &str, day: Option<&str>, fill: bool) {
        let keep_reveal = self.chat_id.as_deref() == Some(chat_id);
        let reveal = if keep_reveal { std::mem::take(&mut self.reveal) } else { HashSet::new() };
        self.reset();
        self.reveal = reveal;
        self.chat_id = Some(chat_id.to_string());
        self.live = true;
        let day = day.map(String::from).unwrap_or_else(today);
        let lines = src.entries(&day);
        let start = lines.len().saturating_sub(BLOCK_LINES);
        let block = self.build_block(src, &day, start, lines.len(), lines[start..].to_vec());
        let loaded = block.lines.len();
        self.blocks.push(block);
        if fill && loaded < MIN_OPEN_LINES {
            self.load_older(src);
        }
        self.update_log_start(src);
        self.scroll = 0;
    }

    fn update_log_start(&mut self, src: &Source) {
        self.log_start = self.older_range(src).is_none();
    }

    pub fn load_older(&mut self, src: &Source) -> bool {
        let Some(older) = self.older_range(src) else {
            self.log_start = true;
            return false;
        };
        self.blocks.insert(0, older);
        self.update_log_start(src);
        self.drop_bottom_over_limit(src);
        true
    }

    fn drop_bottom_over_limit(&mut self, src: &Source) {
        let mut removed_rows = 0;
        let mut total = self.loaded_lines();
        while total > MAX_LOADED_LINES && self.blocks.len() > 1 {
            let block = self.blocks.pop().unwrap();
            total -= block.end - block.start;
            removed_rows += self.block_row_count(&block, src);
            self.live = false;
        }
        self.scroll = self.scroll.saturating_sub(removed_rows);
    }

    pub fn load_newer(&mut self, src: &Source) -> bool {
        let Some(newer) = self.newer_range(src) else {
            self.live = true;
            return false;
        };
        let added = self.block_row_count(&newer, src);
        self.blocks.push(newer);
        self.scroll += added;
        if self.newer_range(src).is_none() {
            self.live = true;
        }
        let mut total = self.loaded_lines();
        while total > MAX_LOADED_LINES && self.blocks.len() > 1 {
            let block = self.blocks.remove(0);
            total -= block.end - block.start;
        }
        self.update_log_start(src);
        true
    }

    /// Adds a new line at the end (only while the newest lines are loaded).
    pub fn append(&mut self, src: &Source, line: &str) {
        if self.chat_id.is_none() || self.blocks.is_empty() || !self.live {
            return;
        }
        let day = line.get(1..11).unwrap_or("").to_string();
        if day.as_str() > self.blocks.last().unwrap().day.as_str() {
            let chat_id = self.chat_id.clone().unwrap();
            self.open(src, &chat_id, Some(&day), false);
            return;
        }
        let rows = self.entry_rows(src, line, None, false, self.width.max(1)).len();
        let last = self.blocks.last_mut().unwrap();
        last.lines.push(line.to_string());
        last.end += 1;
        if self.scroll > 0 {
            self.scroll += rows;
        } else if self.blocks.len() == 1 && self.blocks[0].lines.len() > 2 * BLOCK_LINES {
            let block = &mut self.blocks[0];
            block.lines.drain(..BLOCK_LINES);
            block.start += BLOCK_LINES;
            self.log_start = false;
        }
    }

    pub fn check_day(&mut self, src: &Source) {
        let Some(chat_id) = self.chat_id.clone() else { return };
        let today = today();
        if self.live && self.blocks.last().is_some_and(|block| today > block.day) {
            self.open(src, &chat_id, Some(&today), false);
        }
    }

    /// Scrolls by `rows` (positive: up, towards older lines) and loads more lines at the edges.
    pub fn scroll_by(&mut self, src: &Source, rows: isize) {
        let max = self.total_rows.saturating_sub(self.height);
        if rows > 0 {
            if self.scroll >= max {
                self.load_older(src);
            }
            let max = self.total_rows.saturating_sub(self.height);
            self.scroll = (self.scroll + rows as usize).min(max.max(self.scroll));
        } else {
            self.scroll = self.scroll.saturating_sub(rows.unsigned_abs());
            if self.scroll == 0 && !self.live {
                self.load_newer(src);
            }
        }
    }


    /// Jumps to the newest lines ("now").
    pub fn jump_to_end(&mut self, src: &Source) {
        if let Some(chat_id) = self.chat_id.clone() {
            if !self.live || self.blocks.last().is_some_and(|block| block.day != today()) {
                self.open(src, &chat_id, None, true);
            }
        }
        self.scroll = 0;
    }

    /// Jumps to the beginning of the log.
    pub fn jump_to_start(&mut self, src: &Source) {
        while self.load_older(src) {}
        self.scroll = usize::MAX / 2;
    }

    pub fn reveal_spoilers(&mut self, key: EntryKey) {
        self.reveal.insert(key);
    }

    fn block_rows(&self, block: &Block, src: &Source) -> Vec<Row> {
        let mut rows = Vec::new();
        let width = self.width.max(1);
        if block.start == 0 {
            rows.push(Row { segs: Vec::new(), key: None, kind: RowKind::Blank, images: Vec::new(), slots: Vec::new() });
            let bar = "\u{2500}".repeat(RULE_WIDTH);
            let text = format!("{bar} {} {bar}", block.day);
            let padding = width.saturating_sub(crate::rows::text_width(&text)) / 2;
            let segs = vec![Seg::new(format!("{}{text}", " ".repeat(padding)), dim())];
            rows.push(Row { segs, key: None, kind: RowKind::Rule, images: Vec::new(), slots: Vec::new() });
        }
        for (offset, line) in block.lines.iter().enumerate() {
            let key = (block.day.clone(), block.start + offset);
            let reveal = self.reveal.contains(&key);
            rows.extend(self.entry_rows(src, line, Some(key), reveal, width));
        }
        rows
    }

    /// A single-line message with text and images: the images stand inside the text line.
    fn inline_row(&self, src: &Source, line: &str, key: Option<EntryKey>, reveal: bool, width: usize) -> Option<Row> {
        if line.contains(MARKER) || line.to_ascii_lowercase().contains("[noparse]") {
            return None;
        }
        let (marked, keys) = images::extract_marked(line, images::MAX_PER_MESSAGE, MARKER);
        if keys.is_empty() {
            return None;
        }
        let message = crate::store::parse_line(&marked).map_or(marked.as_str(), |(_, _, message)| message);
        if message.replace(MARKER, "").trim().is_empty() {
            return None;
        }
        let mut segs: Vec<Seg> = Vec::new();
        let mut slots: Vec<(u16, u16)> = Vec::new();
        let mut x = 0usize;
        for seg in render_line(&marked, src.name_style, reveal) {
            if seg.text.contains('\n') {
                return None;
            }
            let mut text = String::new();
            for c in seg.text.chars() {
                if c != MARKER {
                    text.push(c);
                    x += crate::rows::char_width(c);
                    continue;
                }
                if !text.is_empty() {
                    segs.push(Seg { text: std::mem::take(&mut text), style: seg.style, link: seg.link.clone() });
                }
                let columns = (src.image_width)(keys.get(slots.len())?) as usize;
                slots.push((x as u16, columns as u16));
                segs.push(Seg { text: " ".repeat(columns), style: seg.style, link: None });
                x += columns;
            }
            if !text.is_empty() {
                segs.push(Seg { text, style: seg.style, link: seg.link.clone() });
            }
        }
        if x > width || slots.len() != keys.len() {
            return None;
        }
        Some(Row { segs, key, kind: RowKind::Image, images: keys, slots })
    }

    /// Screen rows of one log line, including the strip reserved for its images.
    fn entry_rows(&self, src: &Source, line: &str, key: Option<EntryKey>, reveal: bool, width: usize) -> Vec<Row> {
        if self.image_rows > 0 {
            if let Some(row) = self.inline_row(src, line, key.clone(), reveal, width) {
                let mut rows = vec![row];
                for _ in 1..self.image_rows {
                    rows.push(Row { segs: Vec::new(), key: key.clone(), kind: RowKind::ImageCont, images: Vec::new(), slots: Vec::new() });
                }
                return rows;
            }
        }
        let (text, images) = if self.image_rows > 0 { images::extract(line, images::MAX_PER_MESSAGE) } else { (line.to_string(), Vec::new()) };
        let mut rows: Vec<Row> = wrap(&render_line(&text, src.name_style, reveal), width)
            .into_iter()
            .map(|segs| Row { segs, key: key.clone(), kind: RowKind::Text, images: Vec::new(), slots: Vec::new() })
            .collect();
        if !images.is_empty() {
            rows.push(Row { segs: Vec::new(), key: key.clone(), kind: RowKind::Image, images, slots: Vec::new() });
            for _ in 1..self.image_rows {
                rows.push(Row { segs: Vec::new(), key: key.clone(), kind: RowKind::ImageCont, images: Vec::new(), slots: Vec::new() });
            }
        }
        rows
    }

    fn block_row_count(&self, block: &Block, src: &Source) -> usize {
        self.block_rows(block, src).len()
    }

    /// All rows of the loaded window, oldest first.
    pub fn rows(&mut self, src: &Source, width: usize, height: usize) -> Vec<Row> {
        self.width = width;
        self.height = height;
        let mut rows = Vec::new();
        if self.log_start {
            let text = "Beginning of the log";
            let padding = width.saturating_sub(text.len()) / 2;
            let style = dim().patch(Style::default().add_modifier(ratatui::style::Modifier::ITALIC));
            rows.push(Row { segs: vec![Seg::new(format!("{}{text}", " ".repeat(padding)), style)], key: None, kind: RowKind::LogStart, images: Vec::new(), slots: Vec::new() });
        }
        for block in &self.blocks {
            rows.extend(self.block_rows(block, src));
        }
        self.total_rows = rows.len();
        self.scroll = self.scroll.min(self.total_rows.saturating_sub(height));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bbcode::no_names;
    use crate::store::USER;

    fn setup(name: &str, count: usize) -> (ChatStore, AdSettings, Chat) {
        let dir = std::env::temp_dir().join(format!("tc-pager-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = ChatStore::new("Me", &dir);
        let chat = store.get_or_create("Bob", USER, None).clone();
        for index in 0..count {
            store.append(&chat, "Bob", &format!("line {index}"));
        }
        let ads = AdSettings::load(dir.join("ads.json"), true);
        (store, ads, chat)
    }

    #[test]
    fn opens_with_last_lines_and_a_day_rule() {
        let (store, ads, chat) = setup("open", 5);
        let src = Source { store: &store, ads: &ads, chat: &chat, name_style: &no_names, image_width: &|_| 8 };
        let mut pager = Pager::default();
        pager.open(&src, "user:Bob", None, true);
        let rows = pager.rows(&src, 80, 20);
        assert!(rows.iter().any(|row| row.kind == RowKind::Rule));
        assert_eq!(rows.iter().filter(|row| row.kind == RowKind::Text).count(), 5);
        assert!(pager.log_start);
    }

    #[test]
    fn long_logs_are_windowed_and_load_older_blocks() {
        let (store, ads, chat) = setup("window", 700);
        let src = Source { store: &store, ads: &ads, chat: &chat, name_style: &no_names, image_width: &|_| 8 };
        let mut pager = Pager::default();
        pager.open(&src, "user:Bob", None, true);
        assert_eq!(pager.loaded_lines(), 300);
        pager.rows(&src, 80, 20);
        assert!(pager.load_older(&src));
        assert_eq!(pager.loaded_lines(), 600);
        assert!(pager.load_older(&src));
        assert_eq!(pager.loaded_lines(), 400);
        assert!(!pager.live);
        assert!(pager.load_newer(&src));
        assert_eq!(pager.loaded_lines(), 600);
        assert!(pager.live);
    }

    #[test]
    fn jumps_to_the_start_and_back_to_now() {
        let (store, ads, chat) = setup("jump", 700);
        let src = Source { store: &store, ads: &ads, chat: &chat, name_style: &no_names, image_width: &|_| 8 };
        let mut pager = Pager::default();
        pager.open(&src, "user:Bob", None, true);
        pager.rows(&src, 80, 20);
        pager.jump_to_start(&src);
        let rows = pager.rows(&src, 80, 20);
        assert!(pager.log_start);
        assert!(!pager.live);
        assert_eq!(pager.scroll, pager.total_rows - 20);
        assert!(rows.iter().any(|row| row.kind == RowKind::LogStart));
        pager.jump_to_end(&src);
        pager.rows(&src, 80, 20);
        assert!(pager.live);
        assert_eq!(pager.scroll, 0);
        assert_eq!(pager.loaded_lines(), 300);
    }

    #[test]
    fn append_follows_the_live_end() {
        let (store, ads, chat) = setup("append", 1);
        let src = Source { store: &store, ads: &ads, chat: &chat, name_style: &no_names, image_width: &|_| 8 };
        let mut pager = Pager::default();
        pager.open(&src, "user:Bob", None, true);
        let line = store.append(&chat, "Bob", "neu");
        pager.append(&src, &line);
        assert_eq!(pager.rows(&src, 80, 20).iter().filter(|row| row.kind == RowKind::Text).count(), 2);
    }

    #[test]
    fn images_reserve_rows_below_the_text() {
        let (store, ads, chat) = setup("img", 0);
        let mut pager = Pager { image_rows: 3, ..Default::default() };
        let src = Source { store: &store, ads: &ads, chat: &chat, name_style: &no_names, image_width: &|_| 8 };
        let rows = pager.entry_rows(&src, "[2026-11-18 10:00:00] Max: [eicon]cat[/eicon]", None, false, 40);
        let kinds: Vec<&RowKind> = rows.iter().map(|row| &row.kind).collect();
        assert_eq!(kinds, vec![&RowKind::Text, &RowKind::Image, &RowKind::ImageCont, &RowKind::ImageCont]);
        assert_eq!(rows[1].images.len(), 1);
        assert!(rows[1].slots.is_empty());
        pager.image_rows = 0;
        assert_eq!(pager.entry_rows(&src, "[2026-11-18 10:00:00] Max: hi [eicon]cat[/eicon]", None, false, 40).len(), 1);
    }

    #[test]
    fn images_stand_inside_a_text_line() {
        let (store, ads, chat) = setup("inline", 0);
        let pager = Pager { image_rows: 3, ..Default::default() };
        let src = Source { store: &store, ads: &ads, chat: &chat, name_style: &no_names, image_width: &|_| 8 };
        let rows = pager.entry_rows(&src, "[2026-11-18 10:00:00] Max: hi [eicon]cat[/eicon] da", None, false, 60);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, RowKind::Image);
        assert_eq!(rows[0].slots, vec![(30, 8)]);
        let text: String = rows[0].segs.iter().map(|seg| seg.text.as_str()).collect();
        assert_eq!(text, "[2026-11-18 10:00:00] Max: hi          da");
        let narrow = pager.entry_rows(&src, "[2026-11-18 10:00:00] Max: hi [eicon]cat[/eicon] da", None, false, 30);
        assert!(narrow.iter().any(|row| row.kind == RowKind::Text), "too narrow: strip below the text");
    }
}
