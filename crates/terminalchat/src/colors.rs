//! All colors of the client, configurable in the block `colors` of config.json. Only the values
//! that differ from the defaults need to be there. A style is written like in the Python version
//! (`color_config.py`): words for bold, dim, italic, underline, strike, reverse, a color `#rrggbb`
//! and a background `on #rrggbb`, e.g. "bold #008080". The three backgrounds take a color or an
//! empty string for the terminal's own background (and its transparency).
//!
//! Konsole paints over the pixels of a Sixel image only for cells with a background color of their
//! own, so by default every cell gets one, the same as in the Python version (Textual `textual-dark`).

use std::collections::HashMap;
use std::sync::OnceLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use serde_json::Value;

use crate::style::no_color;

const BACKGROUNDS: [(&str, &str); 5] = [
    ("background", "#121212"),
    ("surface", "#1e1e1e"),
    ("panel", "#242f38"),
    ("status_box", "#242f38"),
    ("status_field", "#1e1e1e"),
];

const STYLES: [(&str, &str); 19] = [
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
    ("online", "#5fd75f"),
    ("offline", "#d75f5f"),
    ("focus", "#5f87d7"),
    ("information", "#5f87d7"),
    ("warning", "#d7af5f"),
    ("error", "#d75f5f"),
    ("ad", "#d7af5f"),
    ("footer", "on #1f2d4a"),
    ("footer_key", "bold #ffb000"),
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

pub const BBCODE: [(&str, &str); 12] = [
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

static PALETTE: OnceLock<Palette> = OnceLock::new();

pub struct Palette {
    backgrounds: HashMap<String, Option<Color>>,
    styles: HashMap<String, Style>,
    status: HashMap<String, Style>,
    gender: HashMap<String, Style>,
    bbcode: HashMap<String, Style>,
    /// Entries of the config that could not be read.
    pub errors: Vec<String>,
}


/// Sets the colors from the block `colors` of the config. Call once at start.
pub fn init(config: Option<&Value>) {
    let _ = PALETTE.set(Palette::new(config));
}


pub fn palette() -> &'static Palette {
    PALETTE.get_or_init(|| Palette::new(None))
}


impl Palette {
    fn new(config: Option<&Value>) -> Palette {
        let mut palette = Palette {
            backgrounds: BACKGROUNDS.iter().map(|(key, value)| (key.to_string(), parse_background(value).flatten())).collect(),
            styles: defaults(&STYLES),
            status: defaults(&STATUS),
            gender: defaults(&GENDER),
            bbcode: defaults(&BBCODE),
            errors: Vec::new(),
        };
        if let Some(Value::Object(entries)) = config {
            for (key, value) in entries {
                palette.set(key, value);
            }
        } else if config.is_some_and(|value| !value.is_null()) {
            palette.errors.push("colors: must be an object".into());
        }
        if no_color() {
            palette.backgrounds.values_mut().for_each(|color| *color = None);
        }
        palette
    }

    fn set(&mut self, key: &str, value: &Value) {
        let map = match key {
            "status" => Some(&mut self.status),
            "gender" => Some(&mut self.gender),
            "bbcode" => Some(&mut self.bbcode),
            _ => None,
        };
        if let Some(map) = map {
            let Value::Object(entries) = value else {
                self.errors.push(format!("colors.{key}: must be an object"));
                return;
            };
            for (name, spec) in entries {
                match spec.as_str().and_then(parse_style) {
                    Some(style) if map.contains_key(name.as_str()) => {
                        map.insert(name.clone(), style);
                    }
                    _ => self.errors.push(format!("colors.{key}.{name}")),
                }
            }
            return;
        }
        let text = value.as_str();
        if self.backgrounds.contains_key(key) {
            match text.and_then(parse_background) {
                Some(color) => {
                    self.backgrounds.insert(key.to_string(), color);
                }
                None => self.errors.push(format!("colors.{key}")),
            }
        } else if self.styles.contains_key(key) {
            match text.and_then(parse_style) {
                Some(style) => {
                    self.styles.insert(key.to_string(), style);
                }
                None => self.errors.push(format!("colors.{key}")),
            }
        } else {
            self.errors.push(format!("colors.{key}: unknown"));
        }
    }

    pub fn style(&self, key: &str) -> Style {
        self.styles.get(key).copied().unwrap_or_default()
    }

    pub fn background(&self, key: &str) -> Option<Color> {
        self.backgrounds.get(key).copied().flatten()
    }

    pub fn status(&self, status: &str) -> Style {
        self.status.get(status).or_else(|| self.status.get("online")).copied().unwrap_or_default()
    }

    pub fn gender(&self, gender: &str) -> Option<Style> {
        self.gender.get(gender).copied()
    }

    pub fn bbcode(&self, name: &str) -> Option<Style> {
        self.bbcode.get(name).copied()
    }
}


fn defaults(table: &[(&str, &str)]) -> HashMap<String, Style> {
    table.iter().map(|(key, spec)| (key.to_string(), parse_style(spec).unwrap_or_default())).collect()
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


fn parse_hex(text: &str) -> Option<Color> {
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
        assert_eq!(palette.style("url"), Style::default().fg(Color::Rgb(255, 0, 0)));
        assert_eq!(palette.style("user"), parse_style("bold #008080").unwrap());
        assert_eq!(palette.status("busy"), Style::default().fg(Color::Rgb(0, 0, 1)));
        assert_eq!(palette.style("eicon"), parse_style("#af5fd7").unwrap());
        assert_eq!(palette.background("status_box"), Some(Color::Rgb(0x24, 0x2f, 0x38)));
        assert_eq!(palette.errors.len(), 3);
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
