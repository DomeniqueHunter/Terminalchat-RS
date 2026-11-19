//! All colors of the client, configurable in the block `colors` of config.json and on the settings
//! page. Only the values that differ from the defaults are stored. A style is written like in the
//! Python version (`color_config.py`): words for bold, dim, italic, underline, strike, reverse, a
//! color `#rrggbb` and a background `on #rrggbb`, e.g. "bold #008080". A background takes a color
//! or an empty string for the terminal's own background (and its transparency).
//!
//! Konsole paints over the pixels of a Sixel image only for cells with a background color of their
//! own, so by default every cell gets one, the same as in the Python version (Textual `textual-dark`).

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock, RwLockReadGuard};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use serde_json::{Map, Value};

use crate::style::no_color;

const BACKGROUNDS: [(&str, &str); 7] = [
    ("background", "#121212"),
    ("surface", "#1e1e1e"),
    ("panel", "#242f38"),
    ("status_box", "#121212"),
    ("status_field", "#121212"),
    ("settings_box", "#121212"),
    ("settings_field", "#1e1e1e"),
];

const TEXT: [(&str, &str); 10] = [
    ("user", "bold #008080"),
    ("icon", "bold #008080"),
    ("eicon", "#af5fd7"),
    ("url", "underline #5f87ff"),
    ("sender", "bold"),
    ("spoiler", "reverse"),
    ("op", "dim"),
    ("channel_link", "bold #5fafff"),
    ("rule", "dim"),
    ("quote", "dim"),
];

const INTERFACE: [(&str, &str); 11] = [
    ("online", "#5fd75f"),
    ("offline", "#d75f5f"),
    ("focus", "#5f87d7"),
    ("information", "#5f87d7"),
    ("warning", "#d7af5f"),
    ("error", "#d75f5f"),
    ("ad", "#d7af5f"),
    ("footer", "on #1f2d4a"),
    ("footer_key", "bold #ffb000"),
    ("settings_selected", "reverse"),
    ("settings_group", "bold"),
];

const STATUS: [(&str, &str); 7] = [
    ("looking", "#55ff55"),
    ("online", "#008080"),
    ("idle", "#a8a8a8"),
    ("away", "#a8a8a8"),
    ("busy", "#ffaf00"),
    ("dnd", "#ff5555"),
    ("crown", "#ffff55"),
];

const GENDER: [(&str, &str); 8] = [
    ("male", "#6699ff"),
    ("female", "#ff6699"),
    ("transgender", "#ee8822"),
    ("herm", "#9b30ff"),
    ("male-herm", "#007fff"),
    ("cunt-boy", "#00cc66"),
    ("shemale", "#cc66ff"),
    ("none", "#ffffbb"),
];

const BBCODE: [(&str, &str); 12] = [
    ("red", "#ff5555"),
    ("blue", "#5f87ff"),
    ("white", "#ffffff"),
    ("yellow", "#ffff55"),
    ("pink", "#ff87d7"),
    ("gray", "#a8a8a8"),
    ("green", "#55ff55"),
    ("orange", "#ffaf00"),
    ("purple", "#af5fff"),
    ("black", "#5f5f5f"),
    ("brown", "#af8700"),
    ("cyan", "#55ffff"),
];

/// The groups that live in a sub object of `colors`, e.g. `"status": {"busy": "#ffaf00"}`.
const NESTED: [&str; 3] = ["status", "gender", "bbcode"];

/// Colors offered by the color picker of the settings page.
pub const PRESETS: [&str; 24] = [
    "#000000", "#121212", "#1e1e1e", "#242f38", "#5f5f5f", "#a8a8a8", "#e0e0e0", "#ffffff",
    "#ff5555", "#d75f5f", "#ffaf00", "#d7af5f", "#ffff55", "#55ff55", "#5fd75f", "#00cc66",
    "#008080", "#55ffff", "#5fafff", "#5f87ff", "#6699ff", "#af5fff", "#af5fd7", "#ff87d7",
];

pub const MODIFIERS: [&str; 6] = ["bold", "dim", "italic", "underline", "strike", "reverse"];

/// A group of the settings page: title, table, kind, prefix of the path.
type Group = (&'static str, &'static [(&'static str, &'static str)], Kind, &'static str);

static PALETTE: LazyLock<RwLock<Palette>> = LazyLock::new(|| RwLock::new(Palette::new(None)));

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Background,
    Style,
}

/// One color on the settings page. `path` is the key, "status.busy" for nested groups.
pub struct Entry {
    pub group: &'static str,
    pub path: String,
    pub kind: Kind,
    pub default: &'static str,
}

pub struct Palette {
    backgrounds: HashMap<String, Option<Color>>,
    styles: HashMap<String, Style>,
    specs: HashMap<String, String>,
    /// Entries of the config that could not be read.
    pub errors: Vec<String>,
}


/// All colors in the order of the settings page.
pub fn entries() -> Vec<Entry> {
    let groups: [Group; 6] = [
        ("Backgrounds", &BACKGROUNDS, Kind::Background, ""),
        ("Text", &TEXT, Kind::Style, ""),
        ("Interface", &INTERFACE, Kind::Style, ""),
        ("Status", &STATUS, Kind::Style, "status."),
        ("Gender", &GENDER, Kind::Style, "gender."),
        ("BBCode", &BBCODE, Kind::Style, "bbcode."),
    ];
    let mut entries = Vec::new();
    for (group, table, kind, prefix) in groups {
        for (key, default) in table {
            entries.push(Entry { group, path: format!("{prefix}{key}"), kind, default });
        }
    }
    entries
}


fn entry(path: &str) -> Option<Entry> {
    entries().into_iter().find(|entry| entry.path == path)
}


/// Sets the colors from the block `colors` of the config. Call at start.
pub fn init(config: Option<&Value>) {
    *PALETTE.write().unwrap_or_else(|error| error.into_inner()) = Palette::new(config);
}


pub fn palette() -> RwLockReadGuard<'static, Palette> {
    PALETTE.read().unwrap_or_else(|error| error.into_inner())
}


/// Checks a value, uses it at once and keeps it in the config block, or drops it from there when
/// it equals the default. The caller saves the config.
pub fn set(colors: &mut Option<Value>, path: &str, spec: &str) -> Result<(), String> {
    let entry = entry(path).ok_or_else(|| format!("unknown color {path}"))?;
    let spec = spec.trim();
    let valid = match entry.kind {
        Kind::Background => parse_background(spec).is_some(),
        Kind::Style => parse_style(spec).is_some(),
    };
    if !valid {
        return Err(match entry.kind {
            Kind::Background => "use #rrggbb or leave it empty".to_string(),
            Kind::Style => "use words like bold, underline, a color #rrggbb, on #rrggbb".to_string(),
        });
    }
    PALETTE.write().unwrap_or_else(|error| error.into_inner()).apply(&entry, spec);
    let is_default = match entry.kind {
        Kind::Background => parse_background(spec) == parse_background(entry.default),
        Kind::Style => parse_style(spec) == parse_style(entry.default),
    };
    store(colors, path, (!is_default).then_some(spec));
    Ok(())
}


/// Back to the default value, which also drops it from the config block.
pub fn reset(colors: &mut Option<Value>, path: &str) {
    if let Some(entry) = entry(path) {
        PALETTE.write().unwrap_or_else(|error| error.into_inner()).apply(&entry, entry.default);
        store(colors, path, None);
    }
}


fn store(colors: &mut Option<Value>, path: &str, spec: Option<&str>) {
    let mut root = match colors.take() {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let (group, key) = match path.split_once('.') {
        Some((group, key)) => (Some(group), key),
        None => (None, path),
    };
    let target = match group {
        Some(group) => {
            let nested = root.entry(group).or_insert_with(|| Value::Object(Map::new()));
            if !nested.is_object() {
                *nested = Value::Object(Map::new());
            }
            nested.as_object_mut().expect("object")
        }
        None => &mut root,
    };
    match spec {
        Some(spec) => {
            target.insert(key.to_string(), Value::String(spec.to_string()));
        }
        None => {
            target.remove(key);
        }
    }
    if let Some(group) = group {
        if root.get(group).and_then(Value::as_object).is_some_and(Map::is_empty) {
            root.remove(group);
        }
    }
    *colors = (!root.is_empty()).then_some(Value::Object(root));
}


impl Palette {
    fn new(config: Option<&Value>) -> Palette {
        let mut palette = Palette { backgrounds: HashMap::new(), styles: HashMap::new(), specs: HashMap::new(), errors: Vec::new() };
        for entry in entries() {
            palette.apply(&entry, entry.default);
        }
        match config {
            Some(Value::Object(map)) => {
                for (key, value) in map {
                    palette.read(key, value);
                }
            }
            Some(Value::Null) | None => {}
            Some(_) => palette.errors.push("colors: must be an object".into()),
        }
        palette
    }

    fn read(&mut self, key: &str, value: &Value) {
        if NESTED.contains(&key) {
            let Value::Object(map) = value else {
                self.errors.push(format!("colors.{key}: must be an object"));
                return;
            };
            for (name, spec) in map {
                self.read_one(&format!("{key}.{name}"), spec);
            }
        } else {
            self.read_one(key, value);
        }
    }

    fn read_one(&mut self, path: &str, value: &Value) {
        let Some(entry) = entry(path) else {
            self.errors.push(format!("colors.{path}: unknown"));
            return;
        };
        let Some(spec) = value.as_str() else {
            self.errors.push(format!("colors.{path}"));
            return;
        };
        let valid = match entry.kind {
            Kind::Background => parse_background(spec).is_some(),
            Kind::Style => parse_style(spec).is_some(),
        };
        if valid {
            self.apply(&entry, spec);
        } else {
            self.errors.push(format!("colors.{path}"));
        }
    }

    fn apply(&mut self, entry: &Entry, spec: &str) {
        match entry.kind {
            Kind::Background => {
                let color = parse_background(spec).flatten().filter(|_| !no_color());
                self.backgrounds.insert(entry.path.clone(), color);
            }
            Kind::Style => {
                self.styles.insert(entry.path.clone(), parse_style(spec).unwrap_or_default());
            }
        }
        self.specs.insert(entry.path.clone(), spec.trim().to_string());
    }

    /// The value as written, e.g. "bold #008080".
    pub fn spec(&self, path: &str) -> &str {
        self.specs.get(path).map(String::as_str).unwrap_or("")
    }

    pub fn style(&self, path: &str) -> Style {
        self.styles.get(path).copied().unwrap_or_default()
    }

    pub fn background(&self, path: &str) -> Option<Color> {
        self.backgrounds.get(path).copied().flatten()
    }

    pub fn status(&self, status: &str) -> Style {
        self.styles.get(&format!("status.{status}")).or_else(|| self.styles.get("status.online")).copied().unwrap_or_default()
    }

    pub fn gender(&self, gender: &str) -> Option<Style> {
        self.styles.get(&format!("gender.{gender}")).copied()
    }

    pub fn bbcode(&self, name: &str) -> Option<Style> {
        self.styles.get(&format!("bbcode.{name}")).copied()
    }
}


/// The parts of a value for the color picker: modifiers, color, background.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Parts {
    pub modifiers: Vec<&'static str>,
    pub color: Option<String>,
    pub background: Option<String>,
}


impl Parts {
    /// Reads a value; for a background the color is the only part.
    pub fn parse(spec: &str, kind: Kind) -> Option<Parts> {
        let mut parts = Parts::default();
        if kind == Kind::Background {
            parse_background(spec)?;
            let spec = spec.trim().to_lowercase();
            parts.color = spec.starts_with('#').then_some(spec);
            return Some(parts);
        }
        let mut words = spec.split_whitespace().map(str::to_lowercase);
        while let Some(word) = words.next() {
            if let Some(modifier) = MODIFIERS.iter().find(|modifier| **modifier == word) {
                if !parts.modifiers.contains(modifier) {
                    parts.modifiers.push(modifier);
                }
            } else if word == "on" {
                let color = words.next()?;
                parse_hex(&color)?;
                parts.background = Some(color);
            } else if word != "default" && word != "none" {
                parse_hex(&word)?;
                parts.color = Some(word);
            }
        }
        Some(parts)
    }

    pub fn has(&self, modifier: &str) -> bool {
        self.modifiers.contains(&modifier)
    }

    pub fn toggle(&mut self, modifier: &'static str) {
        if self.has(modifier) {
            self.modifiers.retain(|present| *present != modifier);
        } else {
            self.modifiers.push(modifier);
        }
    }

    /// Back to text, modifiers in a fixed order: "bold underline #5f87ff on #000000".
    pub fn to_spec(&self, kind: Kind) -> String {
        if kind == Kind::Background {
            return self.color.clone().unwrap_or_default();
        }
        let mut words: Vec<String> = MODIFIERS.iter().filter(|modifier| self.has(modifier)).map(|modifier| modifier.to_string()).collect();
        words.extend(self.color.clone());
        if let Some(background) = &self.background {
            words.push(format!("on {background}"));
        }
        words.join(" ")
    }
}


/// "#rrggbb" gives the color, an empty string (or "default") the terminal's own background.
fn parse_background(text: &str) -> Option<Option<Color>> {
    match text.trim().to_lowercase().as_str() {
        "" | "default" | "none" => Some(None),
        other => parse_hex(other).map(Some),
    }
}


/// Parses a style like "bold underline #5f87ff on #1f2d4a". With NO_COLOR only the attributes
/// stay; a background becomes reverse, so a bar stays visible.
pub fn parse_style(spec: &str) -> Option<Style> {
    let mut style = Style::default();
    let mut words = spec.split_whitespace().map(str::to_lowercase);
    while let Some(word) = words.next() {
        style = match word.as_str() {
            "bold" => style.add_modifier(Modifier::BOLD),
            "dim" => style.add_modifier(Modifier::DIM),
            "italic" => style.add_modifier(Modifier::ITALIC),
            "underline" => style.add_modifier(Modifier::UNDERLINED),
            "strike" => style.add_modifier(Modifier::CROSSED_OUT),
            "reverse" => style.add_modifier(Modifier::REVERSED),
            "default" | "none" => style,
            "on" => {
                let color = parse_hex(&words.next()?)?;
                if no_color() { style.add_modifier(Modifier::REVERSED) } else { style.bg(color) }
            }
            other => {
                let color = parse_hex(other)?;
                if no_color() { style } else { style.fg(color) }
            }
        };
    }
    Some(style)
}


pub fn parse_hex(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8))
}


/// Gives every cell with the default background the main background color.
pub fn apply_background(buffer: &mut Buffer) {
    let Some(color) = palette().background("background") else { return };
    for cell in &mut buffer.content {
        if cell.bg == Color::Reset {
            cell.bg = color;
        }
    }
}


/// Fills an area with one of the backgrounds ("surface", "panel"), if it is set.
pub fn fill(frame: &mut Frame, area: Rect, key: &str) {
    if let Some(color) = palette().background(key) {
        frame.buffer_mut().set_style(area, Style::default().bg(color));
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn styles_are_parsed_like_in_python() {
        assert_eq!(parse_style("bold #008080"), Some(Style::default().add_modifier(Modifier::BOLD).fg(Color::Rgb(0, 0x80, 0x80))));
        assert_eq!(parse_style("on #1f2d4a"), Some(Style::default().bg(Color::Rgb(0x1f, 0x2d, 0x4a))));
        assert_eq!(parse_style("underline"), Some(Style::default().add_modifier(Modifier::UNDERLINED)));
        assert_eq!(parse_style("blinking #fff"), None);
        assert_eq!(parse_style("on"), None);
    }


    #[test]
    fn config_overrides_only_what_it_names() {
        let config = serde_json::json!({
            "background": "",
            "panel": "#010203",
            "url": "#ff0000",
            "status": {"busy": "#000001", "wrong": "#000002"},
            "nonsense": "#000003",
            "eicon": "not a color"
        });
        let palette = Palette::new(Some(&config));
        assert_eq!(palette.background("background"), None);
        assert_eq!(palette.background("panel"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(palette.background("surface"), Some(Color::Rgb(0x1e, 0x1e, 0x1e)));
        assert_eq!(palette.background("status_box"), Some(Color::Rgb(0x12, 0x12, 0x12)));
        assert_eq!(palette.style("url"), Style::default().fg(Color::Rgb(255, 0, 0)));
        assert_eq!(palette.style("user"), parse_style("bold #008080").unwrap());
        assert_eq!(palette.status("busy"), Style::default().fg(Color::Rgb(0, 0, 1)));
        assert_eq!(palette.style("eicon"), parse_style("#af5fd7").unwrap());
        assert_eq!(palette.spec("url"), "#ff0000");
        assert_eq!(palette.errors.len(), 3);
    }


    #[test]
    fn only_values_that_differ_from_the_default_are_stored() {
        let mut colors = None;
        set(&mut colors, "status.busy", "#000001").unwrap();
        set(&mut colors, "url", "underline #5f87ff").unwrap();
        assert_eq!(colors, Some(serde_json::json!({"status": {"busy": "#000001"}})));
        set(&mut colors, "surface", "").unwrap();
        assert_eq!(colors, Some(serde_json::json!({"status": {"busy": "#000001"}, "surface": ""})));
        assert!(set(&mut colors, "url", "#12zz34").is_err());
        assert!(set(&mut colors, "nothing", "#123456").is_err());
        reset(&mut colors, "status.busy");
        reset(&mut colors, "surface");
        assert_eq!(colors, None);
    }


    #[test]
    fn parts_go_back_to_the_same_text() {
        let parts = Parts::parse("underline bold #5f87ff on #000000", Kind::Style).unwrap();
        assert_eq!(parts.modifiers, vec!["underline", "bold"]);
        assert_eq!(parts.to_spec(Kind::Style), "bold underline #5f87ff on #000000");
        let mut plain = Parts::parse("", Kind::Style).unwrap();
        plain.toggle("italic");
        plain.color = Some("#ff0000".into());
        assert_eq!(plain.to_spec(Kind::Style), "italic #ff0000");
        assert_eq!(Parts::parse("#121212", Kind::Background).unwrap().to_spec(Kind::Background), "#121212");
        assert_eq!(Parts::parse("", Kind::Background).unwrap().color, None);
        assert!(Parts::parse("bold #12", Kind::Style).is_none());
    }


    #[test]
    fn every_entry_has_a_readable_default() {
        let entries = entries();
        assert_eq!(entries.len(), 7 + 10 + 11 + 7 + 8 + 12);
        for entry in &entries {
            assert!(Parts::parse(entry.default, entry.kind).is_some(), "{}", entry.path);
        }
    }


    #[test]
    fn only_the_default_background_is_replaced() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer.content[1].bg = Color::Blue;
        apply_background(&mut buffer);
        assert_eq!(buffer.content[0].bg, Color::Rgb(0x12, 0x12, 0x12));
        assert_eq!(buffer.content[1].bg, Color::Blue);
    }
}
