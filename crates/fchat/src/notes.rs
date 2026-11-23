//! F-List notes (messages between characters on the website). There is no documented API for
//! them, so this uses the same calls as the website: a web login with cookies, the JSON calls
//! `notes-get`, `notes-send` and `notes-trash`, and the note page for the text.

use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

const SITE: &str = "https://www.f-list.net";
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Folder {
    Inbox,
    Sent,
}

impl Folder {
    fn id(self) -> u8 {
        match self {
            Folder::Inbox => 1,
            Folder::Sent => 2,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NoteHeader {
    pub id: u64,
    pub title: String,
    pub source: String,
    pub dest: String,
    /// As the site gives it, relative: "2d, 3h ago".
    pub sent: String,
}

/// A logged-in website session. Cheap to clone: the clones share the cookies.
#[derive(Clone)]
pub struct NotesSession {
    client: reqwest::Client,
    /// The characters of the account with their ids, sorted by name.
    pub characters: Vec<(String, u64)>,
}

fn temporary(error: reqwest::Error) -> Error {
    Error::Temporary(format!("f-list.net: {error}"))
}

impl NotesSession {
    /// Logs in on the website with the account and password of the chat.
    pub async fn login(account: &str, password: &str) -> Result<NotesSession> {
        let client = reqwest::Client::builder().cookie_store(true).timeout(TIMEOUT).user_agent("terminalchat").build().map_err(temporary)?;
        let start = client.get(SITE).send().await.map_err(temporary)?.text().await.map_err(temporary)?;
        let token = input_value(&start, "csrf_token").ok_or_else(|| Error::Temporary("f-list.net: no login form found".into()))?;
        let form = [("csrf_token", token.as_str()), ("username", account), ("password", password)];
        let page = client.post(format!("{SITE}/action/script_login.php")).form(&form).send().await.map_err(temporary)?.text().await.map_err(temporary)?;
        let characters = character_data(&page).ok_or_else(|| Error::Rejected("f-list.net: website login failed".into()))?;
        Ok(NotesSession { client, characters })
    }

    /// One page of a folder, newest first, and the number of notes in the folder.
    pub async fn list(&self, folder: Folder, offset: usize, amount: usize) -> Result<(Vec<NoteHeader>, usize)> {
        let url = format!("{SITE}/json/notes-get.json?offset={offset}&amount={amount}&folder={}", folder.id());
        let data: Value = self.client.get(url).send().await.map_err(temporary)?.json().await.map_err(temporary)?;
        check(&data)?;
        Ok(parse_list(&data))
    }

    /// The text of a note as BBCode, read from its page.
    pub async fn text(&self, id: u64) -> Result<String> {
        let page = self.client.get(format!("{SITE}/view_note.php?note_id={id}")).send().await.map_err(temporary)?.text().await.map_err(temporary)?;
        let Some(block) = formatted_block(&page) else {
            let path = std::env::temp_dir().join("terminalchat-note.html");
            let _ = std::fs::write(&path, &page);
            return Err(Error::Temporary(format!("f-list.net: note text not found, page saved to {}", path.display())));
        };
        Ok(html_to_bbcode(block))
    }

    pub async fn send(&self, source: &str, dest: &str, title: &str, message: &str) -> Result<()> {
        let source_id = self.character_id(source).ok_or_else(|| Error::Rejected(format!("{source} is not a character of this account")))?;
        let token = self.token().await?;
        let source_id = source_id.to_string();
        let form = [("csrf_token", token.as_str()), ("title", title), ("message", message), ("dest", dest), ("source", source_id.as_str())];
        let data: Value = self.client.post(format!("{SITE}/json/notes-send.json")).form(&form).send().await.map_err(temporary)?.json().await.map_err(temporary)?;
        check(&data)
    }

    /// Moves notes to the trash.
    pub async fn trash(&self, ids: &[u64]) -> Result<()> {
        let token = self.token().await?;
        let mut form = vec![("csrf_token".to_string(), token)];
        form.extend(ids.iter().map(|id| ("notes[]".to_string(), id.to_string())));
        let data: Value = self.client.post(format!("{SITE}/json/notes-trash.json")).form(&form).send().await.map_err(temporary)?.json().await.map_err(temporary)?;
        check(&data)
    }

    pub fn character_id(&self, name: &str) -> Option<u64> {
        self.characters.iter().find(|(character, _)| character.eq_ignore_ascii_case(name)).map(|(_, id)| *id)
    }

    /// The CSRF token of the JSON calls, from a meta tag of the start page.
    async fn token(&self) -> Result<String> {
        let page = self.client.get(SITE).send().await.map_err(temporary)?.text().await.map_err(temporary)?;
        meta_content(&page, "flcsrf-token").ok_or_else(|| Error::Temporary("f-list.net: no csrf token, session expired?".into()))
    }
}

/// A non-empty "error" in a JSON answer.
fn check(data: &Value) -> Result<()> {
    match data.get("error").and_then(Value::as_str) {
        Some(error) if !error.is_empty() => Err(Error::Rejected(format!("f-list.net: {error}"))),
        _ => Ok(()),
    }
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn number(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn parse_list(data: &Value) -> (Vec<NoteHeader>, usize) {
    let notes = data
        .get("notes")
        .and_then(Value::as_array)
        .map(|notes| {
            notes
                .iter()
                .map(|note| NoteHeader {
                    id: number(note.get("note_id")),
                    title: decode_entities(&text(note.get("title"))),
                    source: text(note.get("source_name")),
                    dest: text(note.get("dest_name")),
                    sent: text(note.get("datetime_sent")),
                })
                .collect()
        })
        .unwrap_or_default();
    (notes, number(data.get("total")) as usize)
}

/// The value of an attribute inside one tag, e.g. `value="..."`.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let end = tag[start..].find('"')?;
    Some(&tag[start..start + end])
}

/// The tags of a page that contain a given text, e.g. `name="csrf_token"`.
fn tags_with<'a>(html: &'a str, needle: &str) -> Vec<&'a str> {
    html.match_indices(needle)
        .filter_map(|(at, _)| {
            let start = html[..at].rfind('<')?;
            let end = at + html[at..].find('>')?;
            Some(&html[start..=end])
        })
        .collect()
}

fn input_value(html: &str, name: &str) -> Option<String> {
    let needle = format!("name=\"{name}\"");
    tags_with(html, &needle).into_iter().find(|tag| tag.starts_with("<input")).and_then(|tag| attribute(tag, "value")).map(String::from)
}

fn meta_content(html: &str, id: &str) -> Option<String> {
    let needle = format!("id=\"{id}\"");
    tags_with(html, &needle).into_iter().find(|tag| tag.starts_with("<meta")).and_then(|tag| attribute(tag, "content")).map(String::from)
}

/// `var characterdata = {...};` of the page after the login: character ids and names.
fn character_data(html: &str) -> Option<Vec<(String, u64)>> {
    let start = html.find("var characterdata")?;
    let start = start + html[start..].find('{')?;
    let end = start + html[start..].find("};")? + 1;
    let data: Value = serde_json::from_str(&quote_number_keys(&html[start..end])).ok()?;
    let mut characters: Vec<(String, u64)> = data
        .as_object()?
        .iter()
        .filter_map(|(key, value)| match value {
            Value::String(name) => Some((decode_entities(name), key.parse().ok()?)),
            Value::Number(id) => Some((key.clone(), id.as_u64()?)),
            _ => None,
        })
        .collect();
    characters.sort_by_key(|(name, _)| name.to_lowercase());
    (!characters.is_empty()).then_some(characters)
}

/// `{123:"Name"}` is JavaScript but not JSON: numbers used as keys get quotes.
fn quote_number_keys(object: &str) -> String {
    let mut out = String::with_capacity(object.len() + 16);
    let chars: Vec<char> = object.chars().collect();
    let mut index = 0;
    let mut in_string = false;
    while index < chars.len() {
        let c = chars[index];
        if in_string {
            out.push(c);
            if c == '\\' && index + 1 < chars.len() {
                out.push(chars[index + 1]);
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c.is_ascii_digit() && out.trim_end().ends_with(['{', ',']) {
            let begin = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }
            let digits: String = chars[begin..index].iter().collect();
            let rest = chars[index..].iter().collect::<String>();
            if rest.trim_start().starts_with(':') {
                out.push('"');
                out.push_str(&digits);
                out.push('"');
            } else {
                out.push_str(&digits);
            }
            continue;
        } else {
            out.push(c);
        }
        index += 1;
    }
    out
}

/// The inside of `<div class="FormattedBlock">`, nested divs included.
fn formatted_block(html: &str) -> Option<&str> {
    let start = html.match_indices("FormattedBlock").find_map(|(at, _)| {
        let open = html[..at].rfind('<')?;
        let close = at + html[at..].find('>')?;
        let tag = &html[open..close];
        let class = tag.find("class=")?;
        (tag.to_lowercase().starts_with("<div") && class < at - open && !tag[..at - open].contains('>')).then_some(close + 1)
    })?;
    let mut depth = 1;
    let mut position = start;
    while depth > 0 {
        let next_open = html[position..].find("<div").map(|at| position + at);
        let next_close = html[position..].find("</div>").map(|at| position + at)?;
        match next_open {
            Some(open) if open < next_close => {
                depth += 1;
                position = open + 4;
            }
            _ => {
                depth -= 1;
                position = next_close + 6;
                if depth == 0 {
                    return Some(&html[start..next_close]);
                }
            }
        }
    }
    None
}

/// Turns the formatted note back into BBCode: line breaks, bold, italic, underline, strike and
/// links stay, other tags go, entities are decoded.
pub fn html_to_bbcode(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut links: Vec<bool> = Vec::new();
    while let Some(open) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..open].replace(['\r', '\n'], "")));
        let Some(close) = rest[open..].find('>') else {
            rest = &rest[open..];
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        let lower = tag.to_lowercase();
        let name = lower.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let closing = lower.starts_with('/');
        let mark = match name {
            "br" => Some("\n".to_string()),
            "p" | "div" if closing => Some("\n".to_string()),
            "b" | "strong" => Some(if closing { "[/b]" } else { "[b]" }.to_string()),
            "i" | "em" => Some(if closing { "[/i]" } else { "[i]" }.to_string()),
            "u" => Some(if closing { "[/u]" } else { "[u]" }.to_string()),
            "s" | "strike" | "del" => Some(if closing { "[/s]" } else { "[s]" }.to_string()),
            "a" if closing => links.pop().and_then(|link| link.then(|| "[/url]".to_string())),
            "a" => {
                let href = attribute(tag, "href").map(decode_entities);
                links.push(href.is_some());
                href.map(|href| format!("[url={href}]"))
            }
            _ => None,
        };
        out.extend(mark);
    }
    out.push_str(&decode_entities(&rest.replace(['\r', '\n'], "")));
    out.trim().to_string()
}

/// The names of the characters 160 to 255 (Latin-1), in order.
const LATIN1: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf", "laquo", "not", "shy", "reg", "macr",
    "deg", "plusmn", "sup2", "sup3", "acute", "micro", "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest",
    "Agrave", "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute", "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml",
    "ETH", "Ntilde", "Ograve", "Oacute", "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute", "THORN", "szlig",
    "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil", "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc", "iuml",
    "eth", "ntilde", "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc", "uuml", "yacute", "thorn", "yuml",
];

/// Named HTML entities beyond the basic five: Latin-1 (umlauts, accents) and common punctuation.
fn named_entity(name: &str) -> Option<char> {
    if let Some(index) = LATIN1.iter().position(|entity| *entity == name) {
        return char::from_u32(160 + index as u32);
    }
    Some(match name {
        "euro" => '\u{20ac}',
        "hellip" => '\u{2026}',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "sbquo" => '\u{201a}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "bdquo" => '\u{201e}',
        "bull" => '\u{2022}',
        "trade" => '\u{2122}',
        "dagger" => '\u{2020}',
        "hearts" => '\u{2665}',
        _ => return None,
    })
}

pub fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32),
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => named_entity(entity),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn the_login_form_and_the_token_are_found() {
        let page = r#"<form><input type="hidden" name="csrf_token" value="abc123"/><input name="username"></form>"#;
        assert_eq!(input_value(page, "csrf_token"), Some("abc123".into()));
        let page = r#"<head><meta name="x" id="flcsrf-token" content="tok-9"></head>"#;
        assert_eq!(meta_content(page, "flcsrf-token"), Some("tok-9".into()));
        assert_eq!(meta_content("<head></head>", "flcsrf-token"), None);
    }


    #[test]
    fn character_data_is_read_with_and_without_quoted_keys() {
        let page = "<script>var characterdata = {123:\"Zed\", 45:\"Anna &amp; Bo\"};</script>";
        assert_eq!(character_data(page), Some(vec![("Anna & Bo".into(), 45), ("Zed".into(), 123)]));
        let page = "<script>var characterdata = {\"7\":\"Solo\"};</script>";
        assert_eq!(character_data(page), Some(vec![("Solo".into(), 7)]));
        assert_eq!(character_data("<p>Wrong password</p>"), None);
    }


    #[test]
    fn a_list_is_read_whatever_type_the_numbers_have() {
        let data = serde_json::json!({
            "error": "",
            "total": "2",
            "notes": [
                {"note_id": "11", "title": "Hi &amp; bye", "source_name": "Anna", "dest_name": "Zed", "folder_id": 1, "datetime_sent": "2d, 3h ago"},
                {"note_id": 12, "title": "Re", "source_name": "Bo", "dest_name": "Solo", "folder_id": "1", "datetime_sent": "5m ago"}
            ]
        });
        let (notes, total) = parse_list(&data);
        assert_eq!(total, 2);
        assert_eq!(notes[0], NoteHeader { id: 11, title: "Hi & bye".into(), source: "Anna".into(), dest: "Zed".into(), sent: "2d, 3h ago".into() });
        assert_eq!(notes[1].id, 12);
        assert!(check(&serde_json::json!({"error": "Not logged in"})).is_err());
        assert!(check(&serde_json::json!({"error": ""})).is_ok());
    }


    #[test]
    fn named_entities_like_umlauts_are_decoded() {
        assert_eq!(decode_entities("w&uuml;rde &Auml;rger, Stra&szlig;e &euro;5 &hellip; &frac12; &bogus; &"), "würde Ärger, Straße €5 … ½ &bogus; &");
    }


    #[test]
    fn the_note_text_comes_back_as_bbcode() {
        let page = r#"<div class="box"><div class="FormattedBlock">Hello <b>you</b>,<br />see <a href="https://example.net/?a=1&amp;b=2">this</a>.<div>inner</div>
        &lt;3 &#x263A;</div></div><div>after</div>"#;
        let block = formatted_block(page).unwrap();
        let other = "<div id='x' class='box FormattedBlock'>one<div>two</div></div><div>no</div>";
        assert_eq!(formatted_block(other), Some("one<div>two</div>"));
        assert_eq!(formatted_block("<p>FormattedBlock</p>"), None);
        assert_eq!(html_to_bbcode(block), "Hello [b]you[/b],\nsee [url=https://example.net/?a=1&b=2]this[/url].inner\n        <3 \u{263A}");
    }
}
