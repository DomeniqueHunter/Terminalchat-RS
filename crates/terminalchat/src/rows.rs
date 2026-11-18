//! Word wrapping of styled segments into screen rows.

use unicode_width::UnicodeWidthChar;

use crate::bbcode::{split_lines, Seg};

pub fn char_width(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

pub fn text_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

fn wrap_line(line: &[Seg], width: usize) -> Vec<Vec<Seg>> {
    let chars: Vec<(char, usize)> = line
        .iter()
        .enumerate()
        .flat_map(|(index, seg)| seg.text.chars().map(move |c| (c, index)))
        .collect();
    if chars.is_empty() {
        return vec![Vec::new()];
    }

    let mut rows: Vec<Vec<(char, usize)>> = Vec::new();
    let mut current: Vec<(char, usize)> = Vec::new();
    let mut current_width = 0;
    let mut index = 0;
    while index < chars.len() {
        let is_space = chars[index].0 == ' ';
        let mut end = index;
        let mut token_width = 0;
        while end < chars.len() && (chars[end].0 == ' ') == is_space {
            token_width += char_width(chars[end].0);
            end += 1;
        }
        if is_space {
            if current_width + token_width <= width {
                current.extend_from_slice(&chars[index..end]);
                current_width += token_width;
            } else if !current.is_empty() {
                rows.push(std::mem::take(&mut current));
                current_width = 0;
            }
        } else if token_width <= width {
            if current_width + token_width > width {
                rows.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.extend_from_slice(&chars[index..end]);
            current_width += token_width;
        } else {
            for &(c, seg_index) in &chars[index..end] {
                let w = char_width(c);
                if current_width + w > width && !current.is_empty() {
                    rows.push(std::mem::take(&mut current));
                    current_width = 0;
                }
                current.push((c, seg_index));
                current_width += w;
            }
        }
        index = end;
    }
    rows.push(current);

    rows.into_iter()
        .map(|row| {
            let mut segs: Vec<Seg> = Vec::new();
            let mut last_index = usize::MAX;
            for (c, seg_index) in row {
                if seg_index == last_index {
                    segs.last_mut().unwrap().text.push(c);
                } else {
                    let source = &line[seg_index];
                    segs.push(Seg { text: c.to_string(), style: source.style, link: source.link.clone() });
                    last_index = seg_index;
                }
            }
            segs
        })
        .collect()
}

/// Wraps segments (which may contain newlines) to rows of at most `width` columns.
pub fn wrap(segs: &[Seg], width: usize) -> Vec<Vec<Seg>> {
    let width = width.max(1);
    split_lines(segs).iter().flat_map(|line| wrap_line(line, width)).collect()
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::*;
    use crate::bbcode::plain_text;

    fn rows(text: &str, width: usize) -> Vec<String> {
        wrap(&[Seg::new(text, Style::default())], width).iter().map(|row| plain_text(row)).collect()
    }

    #[test]
    fn wraps_at_word_boundaries() {
        assert_eq!(rows("hello big world", 9), vec!["hello big", "world"]);
    }

    #[test]
    fn breaks_long_words() {
        assert_eq!(rows("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn keeps_empty_lines() {
        assert_eq!(rows("a\n\nb", 10), vec!["a", "", "b"]);
    }

    #[test]
    fn counts_wide_characters() {
        assert_eq!(rows("日本語日本語", 6), vec!["日本語", "日本語"]);
    }
}
