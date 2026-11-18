//! The multi-line input box: cursor, per-chat history, undo of "clear", templates and
//! tab completion of commands and names.

use std::collections::HashMap;

pub const CURSOR: &str = "{}";
const NAME_WORDS: usize = 3;
const MAX_HISTORY: usize = 100;

#[derive(Debug, Clone)]
struct NameCompletion {
    start: usize,
    names: Vec<String>,
    index: usize,
    text: String,
    cursor: usize,
}

#[derive(Default)]
pub struct InputBox {
    text: String,
    /// Cursor position in characters.
    cursor: usize,
    histories: HashMap<String, Vec<String>>,
    history_key: String,
    history_position: Option<usize>,
    draft: String,
    pub history_enabled: bool,
    cleared: Option<String>,
    completion: Option<NameCompletion>,
}

fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map(|(index, _)| index).unwrap_or(text.len())
}

impl InputBox {
    pub fn new() -> InputBox {
        InputBox { history_enabled: true, ..InputBox::default() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.char_count();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.history_position = None;
        self.completion = None;
    }

    /// Ctrl+C: clears the text but remembers it for `undo_clear`.
    pub fn clear_with_undo(&mut self) {
        if !self.text.is_empty() {
            self.cleared = Some(std::mem::take(&mut self.text));
        }
        self.clear();
    }

    pub fn undo_clear(&mut self) {
        if self.text.is_empty() {
            if let Some(text) = self.cleared.take() {
                self.set_text(&text);
            }
        }
    }

    pub fn insert_str(&mut self, text: &str) {
        let at = byte_index(&self.text, self.cursor);
        self.text.insert_str(at, text);
        self.cursor += text.chars().count();
    }

    pub fn insert_char(&mut self, c: char) {
        self.insert_str(&c.to_string());
    }

    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = byte_index(&self.text, self.cursor - 1);
        let end = byte_index(&self.text, self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let start = byte_index(&self.text, self.cursor);
        let end = byte_index(&self.text, self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_count());
    }

    /// (row, column) of the cursor, in characters.
    pub fn position(&self) -> (usize, usize) {
        let before: String = self.text.chars().take(self.cursor).collect();
        let row = before.matches('\n').count();
        let column = before.rsplit('\n').next().map(|line| line.chars().count()).unwrap_or(0);
        (row, column)
    }

    pub fn line_count(&self) -> usize {
        self.text.matches('\n').count() + 1
    }

    fn line_starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        for (index, c) in self.text.chars().enumerate() {
            if c == '\n' {
                starts.push(index + 1);
            }
        }
        starts
    }

    fn set_position(&mut self, row: usize, column: usize) {
        let starts = self.line_starts();
        let row = row.min(starts.len() - 1);
        let line_end = starts.get(row + 1).map(|next| next - 1).unwrap_or(self.char_count());
        self.cursor = (starts[row] + column).min(line_end);
    }

    pub fn home(&mut self) {
        let (row, _) = self.position();
        self.set_position(row, 0);
    }

    pub fn end(&mut self) {
        let (row, _) = self.position();
        self.set_position(row, usize::MAX / 2);
    }

    pub fn move_up(&mut self) -> bool {
        let (row, column) = self.position();
        if row == 0 {
            return false;
        }
        self.set_position(row - 1, column);
        true
    }

    pub fn move_down(&mut self) -> bool {
        let (row, column) = self.position();
        if row + 1 >= self.line_count() {
            return false;
        }
        self.set_position(row + 1, column);
        true
    }

    // History

    fn sent(&mut self) -> &mut Vec<String> {
        self.histories.entry(self.history_key.clone()).or_default()
    }

    pub fn use_history(&mut self, key: &str) {
        if key != self.history_key {
            self.history_key = key.to_string();
            self.history_position = None;
            self.draft.clear();
        }
    }

    pub fn remember(&mut self, text: &str) {
        if !text.is_empty() {
            let history = self.sent();
            if history.last().map(String::as_str) != Some(text) {
                history.push(text.to_string());
                if history.len() > MAX_HISTORY {
                    history.remove(0);
                }
            }
        }
        self.history_position = None;
        self.draft.clear();
    }

    /// Up: move up a line, or on the first line browse sent texts.
    pub fn history_prev(&mut self) {
        if self.move_up() || !self.history_enabled {
            return;
        }
        let length = self.sent().len();
        if length == 0 {
            return;
        }
        let position = match self.history_position {
            Some(position) => position,
            None => {
                self.draft = self.text.clone();
                length
            }
        };
        if position > 0 {
            self.history_position = Some(position - 1);
            let text = self.sent()[position - 1].clone();
            self.set_text(&text);
        } else {
            self.history_position = Some(position);
        }
    }

    pub fn history_next(&mut self) {
        if self.move_down() || !self.history_enabled {
            return;
        }
        let Some(position) = self.history_position else { return };
        let next = position + 1;
        if next >= self.sent().len() {
            self.history_position = None;
            let draft = self.draft.clone();
            self.set_text(&draft);
        } else {
            self.history_position = Some(next);
            let text = self.sent()[next].clone();
            self.set_text(&text);
        }
    }

    // Templates

    /// Inserts `[b]{}[/b]` style templates; the cursor ends up where `{}` was.
    pub fn insert_template(&mut self, template: &str) {
        match template.split_once(CURSOR) {
            Some((before, after)) => {
                self.insert_str(before);
                let cursor = self.cursor;
                self.insert_str(after);
                self.cursor = cursor;
            }
            None => self.insert_str(template),
        }
    }

    // Completion

    pub fn is_command_word(&self) -> bool {
        self.text.starts_with('/') && !self.text.contains(' ') && !self.text.contains('\n')
    }

    /// Completes the command word as far as it is unambiguous.
    pub fn complete_command(&mut self, names: &[&str]) {
        if names.is_empty() {
            return;
        }
        let completion = if names.len() == 1 {
            format!("{} ", names[0])
        } else {
            common_prefix(names)
        };
        if completion.chars().count() >= self.char_count() - 1 {
            self.set_text(&format!("/{completion}"));
        }
    }

    pub fn in_name_command(&self, name_commands: &[&str], resolve: impl Fn(&str) -> String) -> bool {
        let Some(rest) = self.text.strip_prefix('/') else { return false };
        let command = rest.split(char::is_whitespace).next().unwrap_or("").to_lowercase();
        let resolved = resolve(&command);
        name_commands.contains(&resolved.as_str())
    }

    /// The last 1 to 3 words before the cursor as (start in chars, text).
    fn candidates(&self) -> Vec<(usize, String)> {
        let (row, column) = self.position();
        let starts = self.line_starts();
        let line_start = starts[row];
        let line: Vec<char> = self.text.chars().skip(line_start).take(column).collect();
        if line.last().is_none_or(|c| c.is_whitespace()) {
            return Vec::new();
        }
        let mut word_starts = Vec::new();
        let mut in_word = false;
        for (index, c) in line.iter().enumerate() {
            if c.is_whitespace() {
                in_word = false;
            } else if !in_word {
                word_starts.push(index);
                in_word = true;
            }
        }
        (1..=NAME_WORDS)
            .rev()
            .filter(|count| *count <= word_starts.len())
            .map(|count| {
                let start = word_starts[word_starts.len() - count];
                (line_start + start, line[start..].iter().collect())
            })
            .collect()
    }

    /// Replaces the word before the cursor by a matching name. Returns the notification text
    /// (position in the list) when there are several matches.
    pub fn complete_name(
        &mut self,
        step: isize,
        in_name_command: bool,
        source: impl Fn(&str) -> Vec<String>,
    ) -> Option<String> {
        let current = self.completion.clone().filter(|state| state.text == self.text && state.cursor == self.cursor);
        let mut state = match current {
            Some(mut state) => {
                let length = state.names.len() as isize;
                state.index = ((state.index as isize + step).rem_euclid(length)) as usize;
                state
            }
            None => {
                let found = self.candidates().into_iter().find_map(|(start, prefix)| {
                    let names = source(&prefix);
                    (!names.is_empty()).then_some((start, names))
                });
                let Some((start, names)) = found else {
                    self.completion = None;
                    return None;
                };
                let index = if step > 0 { 0 } else { names.len() - 1 };
                NameCompletion { start, names, index, text: String::new(), cursor: self.cursor }
            }
        };
        let name = state.names[state.index].clone();
        let insert = if in_name_command { name.clone() } else { format!("[user]{name}[/user]") };
        let from = byte_index(&self.text, state.start);
        let to = byte_index(&self.text, state.cursor);
        self.text.replace_range(from..to, &insert);
        self.cursor = state.start + insert.chars().count();
        state.text = self.text.clone();
        state.cursor = self.cursor;
        let note = (state.names.len() > 1).then(|| format!("{}/{}: {}", state.index + 1, state.names.len(), name));
        self.completion = Some(state);
        note
    }
}

fn common_prefix(names: &[&str]) -> String {
    let first = names[0];
    let mut length = first.len();
    for name in &names[1..] {
        length = length.min(first.bytes().zip(name.bytes()).take_while(|(a, b)| a == b).count());
    }
    first[..length].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: &str) -> InputBox {
        let mut input = InputBox::new();
        input.set_text(text);
        input
    }

    #[test]
    fn edits_text_with_umlauts() {
        let mut input = input("äö");
        input.left();
        input.insert_char('x');
        assert_eq!(input.text(), "äxö");
        input.backspace();
        input.delete();
        assert_eq!(input.text(), "ä");
    }

    #[test]
    fn moves_between_lines() {
        let mut input = input("abc\nde");
        assert_eq!(input.position(), (1, 2));
        assert!(input.move_up());
        assert_eq!(input.position(), (0, 2));
        assert!(!input.move_up());
        input.end();
        assert_eq!(input.position(), (0, 3));
    }

    #[test]
    fn browses_history_per_chat() {
        let mut input = InputBox::new();
        input.use_history("a");
        input.remember("eins");
        input.remember("zwei");
        input.set_text("entwurf");
        input.history_prev();
        assert_eq!(input.text(), "zwei");
        input.history_prev();
        assert_eq!(input.text(), "eins");
        input.history_next();
        input.history_next();
        assert_eq!(input.text(), "entwurf");
        input.use_history("b");
        input.set_text("");
        input.history_prev();
        assert_eq!(input.text(), "");
    }

    #[test]
    fn clear_can_be_undone() {
        let mut input = input("wichtig");
        input.clear_with_undo();
        assert_eq!(input.text(), "");
        input.undo_clear();
        assert_eq!(input.text(), "wichtig");
    }

    #[test]
    fn templates_place_the_cursor() {
        let mut input = InputBox::new();
        input.insert_template("[b]{}[/b]");
        input.insert_str("hi");
        assert_eq!(input.text(), "[b]hi[/b]");
    }

    #[test]
    fn completes_commands() {
        let mut input = input("/jo");
        input.complete_command(&["join"]);
        assert_eq!(input.text(), "/join ");
        let mut input2 = super::tests::input("/s");
        input2.complete_command(&["status", "setmode", "setowner"]);
        assert_eq!(input2.text(), "/s");
    }

    #[test]
    fn completes_names_with_several_words_and_cycles() {
        let source = |prefix: &str| -> Vec<String> {
            ["Max Mustermann", "Maxine"].iter().filter(|n| n.to_lowercase().starts_with(&prefix.to_lowercase())).map(|n| n.to_string()).collect()
        };
        let mut input = input("hi max mu");
        let note = input.complete_name(1, false, source);
        assert_eq!(input.text(), "hi [user]Max Mustermann[/user]");
        assert!(note.is_none());

        let mut input = super::tests::input("hi ma");
        let note = input.complete_name(1, false, source);
        assert_eq!(input.text(), "hi [user]Max Mustermann[/user]");
        assert_eq!(note.as_deref(), Some("1/2: Max Mustermann"));
        input.complete_name(1, false, source);
        assert_eq!(input.text(), "hi [user]Maxine[/user]");
        input.complete_name(-1, false, source);
        assert_eq!(input.text(), "hi [user]Max Mustermann[/user]");
    }

    #[test]
    fn names_in_commands_are_plain() {
        let mut input = input("/priv ma");
        input.complete_name(1, true, |prefix| if prefix == "ma" { vec!["Maxine".to_string()] } else { Vec::new() });
        assert_eq!(input.text(), "/priv Maxine");
    }
}
