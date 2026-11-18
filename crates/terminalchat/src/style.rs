//! Colors and styles. `NO_COLOR` removes all colors but keeps bold, italic and underline.

use std::sync::OnceLock;

use ratatui::style::{Modifier, Style};

pub fn no_color() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()))
}

pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

pub fn italic() -> Style {
    Style::default().add_modifier(Modifier::ITALIC)
}

pub fn underlined() -> Style {
    Style::default().add_modifier(Modifier::UNDERLINED)
}

pub fn strike() -> Style {
    Style::default().add_modifier(Modifier::CROSSED_OUT)
}

pub fn reversed() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// A style from the color config, see `colors.rs`.
pub fn color(key: &str) -> Style {
    crate::colors::palette().style(key)
}

pub fn user_style() -> Style {
    color("user")
}

pub fn icon_style() -> Style {
    color("icon")
}

pub fn eicon_style() -> Style {
    color("eicon")
}

pub fn url_style() -> Style {
    color("url")
}

pub fn sender_style() -> Style {
    color("sender")
}

pub fn spoiler_style() -> Style {
    color("spoiler")
}

pub fn channel_link_style() -> Style {
    color("channel_link")
}

pub fn quote_style() -> Style {
    color("quote")
}

pub fn status_symbol(status: &str) -> &'static str {
    match status {
        "looking" => "+",
        "idle" | "away" => "~",
        "busy" => "-",
        "dnd" => "x",
        "crown" => "*",
        _ => "\u{2022}",
    }
}

pub fn status_style(status: &str) -> Style {
    crate::colors::palette().status(status)
}

pub fn gender_style(gender: Option<&str>) -> Style {
    gender.and_then(|gender| crate::colors::palette().gender(&gender.to_lowercase())).unwrap_or_default()
}

pub const BBCODE_COLOR_NAMES: [&str; 12] =
    ["red", "blue", "white", "yellow", "pink", "gray", "green", "orange", "purple", "black", "brown", "cyan"];

pub fn bbcode_color(name: &str) -> Option<Style> {
    crate::colors::palette().bbcode(&name.to_lowercase())
}
