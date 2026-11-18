//! Colors and styles. `NO_COLOR` removes all colors but keeps bold, italic and underline.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};

pub const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn no_color() -> bool {
    static VALUE: OnceLock<bool> = OnceLock::new();
    *VALUE.get_or_init(|| std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()))
}

pub fn fg(color: Color) -> Style {
    if no_color() {
        Style::default()
    } else {
        Style::default().fg(color)
    }
}

pub fn on(color: Color) -> Style {
    if no_color() {
        reversed()
    } else {
        Style::default().bg(color)
    }
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

pub fn user_style() -> Style {
    fg(rgb(0x008080)).patch(bold())
}

pub fn icon_style() -> Style {
    user_style()
}

pub fn eicon_style() -> Style {
    fg(rgb(0xaf5fd7))
}

pub fn url_style() -> Style {
    fg(rgb(0x5f87ff)).patch(underlined())
}

pub fn sender_style() -> Style {
    bold()
}

pub fn spoiler_style() -> Style {
    reversed()
}

pub fn channel_link_style() -> Style {
    fg(rgb(0x5fafff)).patch(bold())
}

pub fn quote_style() -> Style {
    dim()
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
    match status {
        "looking" => fg(rgb(0x55ff55)),
        "idle" | "away" => fg(rgb(0xa8a8a8)),
        "busy" => fg(rgb(0xffaf00)),
        "dnd" => fg(rgb(0xff5555)),
        "crown" => fg(rgb(0xffff55)),
        _ => fg(rgb(0x008080)),
    }
}

pub fn gender_style(gender: Option<&str>) -> Style {
    let color = match gender.map(str::to_lowercase).as_deref() {
        Some("male") => 0x6699ff,
        Some("female") => 0xff6699,
        Some("transgender") => 0xee8822,
        Some("herm") => 0x9b30ff,
        Some("male-herm") => 0x007fff,
        Some("cunt-boy") => 0x00cc66,
        Some("shemale") => 0xcc66ff,
        Some("none") => 0xffffbb,
        _ => return Style::default(),
    };
    fg(rgb(color))
}

pub const BBCODE_COLOR_NAMES: [&str; 12] =
    ["red", "blue", "white", "yellow", "pink", "gray", "green", "orange", "purple", "black", "brown", "cyan"];

pub fn bbcode_color(name: &str) -> Option<Style> {
    let color = match name.to_lowercase().as_str() {
        "red" => 0xff5555,
        "blue" => 0x5f87ff,
        "white" => 0xffffff,
        "yellow" => 0xffff55,
        "pink" => 0xff87d7,
        "gray" => 0xa8a8a8,
        "green" => 0x55ff55,
        "orange" => 0xffaf00,
        "purple" => 0xaf5fff,
        "black" => 0x5f5f5f,
        "brown" => 0xaf8700,
        "cyan" => 0x55ffff,
        _ => return None,
    };
    Some(fg(rgb(color)))
}
