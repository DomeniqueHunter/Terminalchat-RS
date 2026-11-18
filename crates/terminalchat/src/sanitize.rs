//! Cleans everything that comes from the server before it is shown or logged.

use serde_json::Value;

fn is_unsafe(c: char) -> bool {
    matches!(c,
        '\u{00}'..='\u{08}' | '\u{0b}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Removes control characters (terminal escapes, C1 codes) and Unicode direction overrides.
/// Newline and tab are kept.
pub fn clean_text(text: &str) -> String {
    text.chars().filter(|c| !is_unsafe(*c)).collect()
}

pub fn clean_data(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(clean_text(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(clean_data).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(key, item)| (key, clean_data(item))).collect()),
        other => other,
    }
}

/// Makes a name usable as a single path component.
pub fn safe_path_part(name: &str) -> String {
    let part = clean_text(name).replace(['/', '\\'], "_");
    let part = part.trim();
    if part.is_empty() || part.starts_with('.') {
        format!("_{part}")
    } else {
        part.to_string()
    }
}

pub fn is_safe_url(url: &str) -> bool {
    let lowered = url.trim().to_lowercase();
    lowered.starts_with("http://") || lowered.starts_with("https://")
}

/// Decodes the HTML entities the server may send (`&amp;`, `&#39;`, ...).
pub fn html_unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => entity.strip_prefix('#').and_then(|number| {
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => number.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }),
            };
            character.map(|c| (c, end + 1))
        });
        match decoded {
            Some((character, length)) => {
                result.push(character);
                rest = &rest[length..];
            }
            None => {
                result.push('&');
                rest = &rest[1..];
            }
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_escape_sequences_and_overrides() {
        let dirty = "a\u{1b}]52;c;SGFjaw==\u{1b}\\b\u{202e}c\u{9b}2J\n\td";
        assert_eq!(clean_text(dirty), "a]52;c;SGFjaw==\\bc2J\n\td");
    }

    #[test]
    fn makes_safe_path_parts() {
        assert_eq!(safe_path_part("../../etc"), "_.._.._etc");
        assert_eq!(safe_path_part("ADH-123"), "ADH-123");
        assert_eq!(safe_path_part("a/b"), "a_b");
        assert_eq!(safe_path_part(""), "_");
    }

    #[test]
    fn only_http_urls_are_safe() {
        assert!(is_safe_url("https://f-list.net"));
        assert!(!is_safe_url("file:///etc/passwd"));
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(html_unescape("a &amp; b &lt;3 &#39;x&#x27; &unknown; &"), "a & b <3 'x' &unknown; &");
    }
}
