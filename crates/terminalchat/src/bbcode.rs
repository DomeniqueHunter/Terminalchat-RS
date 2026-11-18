//! BBCode to styled text segments. Segments can carry a link (user, channel, spoiler or URL).

use ratatui::style::Style;

use crate::style::{
    bbcode_color, bold, channel_link_style, dim, eicon_style, icon_style, italic, quote_style, reversed,
    spoiler_style, strike, underlined, url_style, user_style,
};

pub const USER_LINK: &str = "fchat-user:";
pub const CHANNEL_LINK: &str = "fchat-channel:";
pub const SPOILER_LINK: &str = "fchat-spoiler:";
pub const QUOTE_BAR: &str = "\u{2502} ";
pub const SPOILER_LABEL: &str = " spoiler ";
const RULE: &str = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";

pub const EICON_URL: &str = "https://static.f-list.net/images/eicon/";
pub const PROFILE_URL: &str = "https://www.f-list.net/c/";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Seg {
    pub text: String,
    pub style: Style,
    pub link: Option<String>,
}

impl Seg {
    pub fn new(text: impl Into<String>, style: Style) -> Seg {
        Seg { text: text.into(), style, link: None }
    }

    pub fn linked(text: impl Into<String>, style: Style, link: impl Into<String>) -> Seg {
        Seg { text: text.into(), style, link: Some(link.into()) }
    }
}

pub type NameStyle<'a> = &'a dyn Fn(&str) -> Style;

#[cfg(test)]
pub fn plain_text(segs: &[Seg]) -> String {
    segs.iter().map(|seg| seg.text.as_str()).collect()
}

pub fn percent_encode(text: &str) -> String {
    let mut result = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

pub fn eicon_url(name: &str) -> String {
    format!("{EICON_URL}{}.gif", percent_encode(&name.trim().to_lowercase()))
}

pub fn profile_url(name: &str) -> String {
    format!("{PROFILE_URL}{}", percent_encode(name.trim()))
}

struct Tag<'a> {
    start: usize,
    end: usize,
    closing: bool,
    name: String,
    value: Option<&'a str>,
}

fn next_tag(source: &str, from: usize) -> Option<Tag<'_>> {
    let mut search = from;
    while let Some(offset) = source[search..].find('[') {
        let start = search + offset;
        search = start + 1;
        let rest = &source[start + 1..];
        let close = rest.find(']')?;
        let inner = &rest[..close];
        let (closing, body) = match inner.strip_prefix('/') {
            Some(body) => (true, body),
            None => (false, inner),
        };
        let (name, value) = match body.split_once('=') {
            Some((name, value)) if !closing => (name, Some(value)),
            _ => (body, None),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        return Some(Tag { start, end: start + 1 + close + 1, closing, name: name.to_lowercase(), value });
    }
    None
}

fn find_close(source: &str, name: &str, from: usize) -> Option<usize> {
    let lowered = source.to_ascii_lowercase();
    lowered[from..].find(&format!("[/{name}]")).map(|index| from + index)
}

fn simple_style(name: &str) -> Option<Style> {
    Some(match name {
        "b" | "big" => bold(),
        "i" => italic(),
        "u" => underlined(),
        "s" => strike(),
        "sub" | "sup" | "small" => dim(),
        "heading" => bold().patch(underlined()),
        "center" | "left" | "right" | "justify" | "indent" | "collapse" | "img" => Style::default(),
        _ => return None,
    })
}

const CONTENT_TAGS: [&str; 9] = ["user", "icon", "eicon", "url", "noparse", "session", "channel", "spoiler", "quote"];

fn push(segs: &mut Vec<Seg>, text: &str, style: Style) {
    if !text.is_empty() {
        segs.push(Seg::new(text, style));
    }
}

/// Splits segments at newlines into lines of segments.
pub fn split_lines(segs: &[Seg]) -> Vec<Vec<Seg>> {
    let mut lines: Vec<Vec<Seg>> = vec![Vec::new()];
    for seg in segs {
        for (index, part) in seg.text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            if !part.is_empty() {
                lines.last_mut().unwrap().push(Seg { text: part.to_string(), style: seg.style, link: seg.link.clone() });
            }
        }
    }
    lines
}

fn ends_with_newline(segs: &[Seg]) -> bool {
    segs.iter().rev().find(|seg| !seg.text.is_empty()).is_some_and(|seg| seg.text.ends_with('\n'))
}

fn append_quote(segs: &mut Vec<Seg>, quoted: &[Seg], base: Style, author: Option<&str>) {
    if !segs.is_empty() && !ends_with_newline(segs) {
        push(segs, "\n", Style::default());
    }
    let bar = base.patch(quote_style());
    if let Some(author) = author {
        push(segs, QUOTE_BAR, bar);
        push(segs, &format!("{author} wrote:"), bar.patch(italic()));
        push(segs, "\n", Style::default());
    }
    for line in split_lines(quoted) {
        push(segs, QUOTE_BAR, bar);
        segs.extend(line);
        push(segs, "\n", Style::default());
    }
}

fn render_content_tag(
    segs: &mut Vec<Seg>,
    name: &str,
    value: Option<&str>,
    content: &str,
    base: Style,
    name_style: NameStyle,
    reveal: bool,
) {
    match name {
        "user" => {
            let style = base.patch(user_style()).patch(name_style(content));
            segs.push(Seg::linked(content, style, format!("{USER_LINK}{}", content.trim())));
        }
        "icon" => {
            let style = base.patch(icon_style()).patch(name_style(content));
            segs.push(Seg::linked(format!("[{content}]"), style, profile_url(content)));
        }
        "eicon" => {
            segs.push(Seg::linked(format!(":{content}:"), base.patch(eicon_style()), eicon_url(content)));
        }
        "session" => {
            let title = value.unwrap_or(content);
            segs.push(Seg::linked(
                format!("#{title}"),
                base.patch(channel_link_style()),
                format!("{CHANNEL_LINK}{}", content.trim()),
            ));
        }
        "channel" => {
            segs.push(Seg::linked(
                format!("#{content}"),
                base.patch(channel_link_style()),
                format!("{CHANNEL_LINK}{}", content.trim()),
            ));
        }
        "spoiler" => {
            if reveal {
                segs.extend(to_segs(content, name_style, true).into_iter().map(|mut seg| {
                    seg.style = base.patch(seg.style);
                    seg
                }));
            } else {
                segs.push(Seg::linked(SPOILER_LABEL, base.patch(spoiler_style()), SPOILER_LINK));
            }
        }
        "quote" => {
            let quoted: Vec<Seg> = to_segs(content.trim_matches('\n'), name_style, reveal)
                .into_iter()
                .map(|mut seg| {
                    seg.style = base.patch(seg.style);
                    seg
                })
                .collect();
            append_quote(segs, &quoted, base, value);
        }
        "url" => {
            let url = value.unwrap_or(content);
            let label = if content.is_empty() { url } else { content };
            segs.push(Seg::linked(label, base.patch(url_style()), url));
        }
        _ => push(segs, content, base),
    }
}

/// Renders BBCode. Unknown or unbalanced tags are kept as text.
pub fn to_segs(source: &str, name_style: NameStyle, reveal_spoilers: bool) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    let mut stack: Vec<(String, Style)> = Vec::new();
    let mut position = 0;

    let current = |stack: &[(String, Style)]| stack.iter().fold(Style::default(), |acc, (_, style)| acc.patch(*style));

    while position < source.len() {
        let Some(tag) = next_tag(source, position) else {
            push(&mut segs, &source[position..], current(&stack));
            break;
        };
        push(&mut segs, &source[position..tag.start], current(&stack));
        position = tag.end;
        let raw = &source[tag.start..tag.end];

        if !tag.closing && CONTENT_TAGS.contains(&tag.name.as_str()) {
            if let Some(end) = find_close(source, &tag.name, position) {
                render_content_tag(
                    &mut segs,
                    &tag.name,
                    tag.value,
                    &source[position..end],
                    current(&stack),
                    name_style,
                    reveal_spoilers,
                );
                position = end + tag.name.len() + 3;
                if tag.name == "quote" && source[position..].starts_with('\n') {
                    position += 1;
                }
                continue;
            }
            push(&mut segs, raw, current(&stack));
            continue;
        }

        if tag.name == "hr" {
            if !tag.closing {
                push(&mut segs, RULE, dim());
            }
            continue;
        }

        if tag.closing {
            match stack.iter().rposition(|(name, _)| *name == tag.name) {
                Some(index) => stack.truncate(index),
                None => push(&mut segs, raw, current(&stack)),
            }
            continue;
        }

        let style = if tag.name == "color" {
            tag.value.map(|value| bbcode_color(value).unwrap_or_default())
        } else {
            simple_style(&tag.name)
        };
        match style {
            Some(style) => stack.push((tag.name, style)),
            None => push(&mut segs, raw, current(&stack)),
        }
    }

    if ends_with_newline(&segs) && !source.ends_with('\n') {
        if let Some(last) = segs.iter_mut().rev().find(|seg| !seg.text.is_empty()) {
            last.text.pop();
        }
    }
    segs
}

#[cfg(test)]
pub fn no_names(_: &str) -> Style {
    Style::default()
}

#[allow(dead_code)]
pub fn reverse_only() -> Style {
    reversed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(source: &str) -> String {
        plain_text(&to_segs(source, &no_names, false))
    }

    #[test]
    fn renders_basic_tags() {
        assert_eq!(plain("Hi [b]fett [i]kursiv[/i][/b]!"), "Hi fett kursiv!");
    }

    #[test]
    fn keeps_unknown_and_unbalanced_tags() {
        assert_eq!(plain("kaputt [/b] und [b]offen [foo]x[/foo]"), "kaputt [/b] und offen [foo]x[/foo]");
    }

    #[test]
    fn noparse_keeps_tags() {
        assert_eq!(plain("[noparse][b]roh[/b][/noparse]"), "[b]roh[/b]");
    }

    #[test]
    fn spoilers_are_hidden_until_revealed() {
        assert_eq!(plain("Geheim: [spoiler]Butler[/spoiler]"), "Geheim:  spoiler ");
        assert_eq!(plain_text(&to_segs("[spoiler]Butler[/spoiler]", &no_names, true)), "Butler");
    }

    #[test]
    fn user_and_channel_links() {
        let segs = to_segs("[user]Max Mustermann[/user] [session=Mein Raum]ADH-1[/session]", &no_names, false);
        assert_eq!(segs[0].link.as_deref(), Some("fchat-user:Max Mustermann"));
        assert_eq!(segs[2].text, "#Mein Raum");
        assert_eq!(segs[2].link.as_deref(), Some("fchat-channel:ADH-1"));
    }

    #[test]
    fn quotes_get_a_bar_and_author() {
        let text = plain("[quote=Alice]eins\nzwei[/quote]\nAntwort");
        assert_eq!(text, "\u{2502} Alice wrote:\n\u{2502} eins\n\u{2502} zwei\nAntwort");
    }

    #[test]
    fn urls_use_their_label() {
        let segs = to_segs("[url=https://f-list.net]f-list[/url]", &no_names, false);
        assert_eq!(segs[0].text, "f-list");
        assert_eq!(segs[0].link.as_deref(), Some("https://f-list.net"));
    }

    #[test]
    fn eicon_url_is_lowercase_and_encoded() {
        assert_eq!(eicon_url("Cat Hug"), "https://static.f-list.net/images/eicon/cat%20hug.gif");
    }
}
