//! Modal screens: login, character choice, profile, info, pickers and a one-line editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;

use crate::bbcode::Seg;
use crate::input::InputBox;
use crate::style::BBCODE_COLOR_NAMES;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerKind {
    Emoji,
    Bbcode,
    Channels,
    Commands,
}

pub enum Action {
    None,
    Close,
    Login { account: String, password: String, remember: bool },
    Character(String),
    Pick(PickerKind, String),
    Edit(String),
}

pub enum Screen {
    Login(LoginScreen),
    Character(CharacterScreen),
    Profile(ProfileScreen),
    Info(InfoScreen),
    Picker(PickerScreen),
    Edit(EditScreen),
}

impl Screen {
    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match self {
            Screen::Login(screen) => screen.on_key(key),
            Screen::Character(screen) => screen.on_key(key),
            Screen::Profile(screen) => screen.on_key(key),
            Screen::Info(screen) => screen.on_key(key),
            Screen::Picker(screen) => screen.on_key(key),
            Screen::Edit(screen) => screen.on_key(key),
        }
    }

    /// Scrolls by mouse wheel (positive: down).
    pub fn scroll(&mut self, rows: isize) {
        match self {
            Screen::Profile(screen) => screen.scroll = screen.scroll.saturating_add_signed(rows),
            Screen::Info(screen) => screen.scroll = screen.scroll.saturating_add_signed(rows),
            Screen::Picker(screen) => screen.move_selection(rows),
            Screen::Character(screen) => screen.move_selection(rows),
            _ => {}
        }
    }
}

fn is_ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Splits text into one segment line per text line.
pub fn text_lines(text: &str, style: Style) -> Vec<Vec<Seg>> {
    text.split('\n').map(|line| vec![Seg::new(line, style)]).collect()
}

// ------------------------------------------------------------------ login

pub struct LoginScreen {
    pub account: String,
    pub password: String,
    pub remember: bool,
    pub focus: usize,
    pub message: String,
    pub busy: bool,
}

impl LoginScreen {
    pub fn new(account: &str, remember: bool, message: &str) -> LoginScreen {
        LoginScreen {
            account: account.to_string(),
            password: String::new(),
            remember,
            focus: if account.is_empty() { 0 } else { 1 },
            message: message.to_string(),
            busy: false,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        if self.busy {
            return Action::None;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % 3,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + 2) % 3,
            KeyCode::Enter => {
                if self.account.trim().is_empty() || self.password.is_empty() {
                    self.message = "Account and password are required".into();
                    return Action::None;
                }
                self.busy = true;
                self.message = "Logging in...".into();
                return Action::Login {
                    account: self.account.trim().to_string(),
                    password: self.password.clone(),
                    remember: self.remember,
                };
            }
            KeyCode::Backspace => {
                match self.focus {
                    0 => self.account.pop(),
                    1 => self.password.pop(),
                    _ => None,
                };
            }
            KeyCode::Char(' ') if self.focus == 2 => self.remember = !self.remember,
            KeyCode::Char(c) if !is_ctrl(&key) => match self.focus {
                0 => self.account.push(c),
                1 => self.password.push(c),
                _ => {}
            },
            _ => {}
        }
        Action::None
    }
}

// ------------------------------------------------------------------ character

pub struct CharacterScreen {
    pub names: Vec<String>,
    pub selected: usize,
}

impl CharacterScreen {
    pub fn new(names: Vec<String>, preferred: &str) -> CharacterScreen {
        let selected = names.iter().position(|name| name.eq_ignore_ascii_case(preferred)).unwrap_or(0);
        CharacterScreen { names, selected }
    }

    fn move_selection(&mut self, step: isize) {
        if !self.names.is_empty() {
            self.selected = (self.selected as isize + step).clamp(0, self.names.len() as isize - 1) as usize;
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.names.len().saturating_sub(1),
            KeyCode::Enter => {
                if let Some(name) = self.names.get(self.selected) {
                    return Action::Character(name.clone());
                }
            }
            KeyCode::Esc => return Action::Close,
            _ => {}
        }
        Action::None
    }
}

// ------------------------------------------------------------------ profile

pub struct ProfileScreen {
    pub character: String,
    pub result: Option<Result<fchat::Profile, String>>,
    pub scroll: usize,
}

impl ProfileScreen {
    pub fn new(character: &str) -> ProfileScreen {
        ProfileScreen { character: character.to_string(), result: None, scroll: 0 }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll += 1,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(15),
            KeyCode::PageDown => self.scroll += 15,
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = usize::MAX / 2,
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            _ => {}
        }
        Action::None
    }
}

// ------------------------------------------------------------------ info

pub struct InfoScreen {
    pub title: String,
    pub lines: Vec<Vec<Seg>>,
    pub wide: bool,
    pub scroll: usize,
}

impl InfoScreen {
    pub fn new(title: &str, lines: Vec<Vec<Seg>>, wide: bool) -> InfoScreen {
        InfoScreen { title: title.to_string(), lines, wide, scroll: 0 }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll += 1,
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(15),
            KeyCode::PageDown => self.scroll += 15,
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = usize::MAX / 2,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => return Action::Close,
            _ => {}
        }
        Action::None
    }
}

// ------------------------------------------------------------------ picker

pub struct PickItem {
    pub label: String,
    pub value: String,
    pub hint: String,
}

pub struct PickerScreen {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<PickItem>,
    pub filter: String,
    pub selected: usize,
}

impl PickerScreen {
    pub fn new(kind: PickerKind, title: &str, items: Vec<PickItem>, filter: &str) -> PickerScreen {
        PickerScreen { kind, title: title.to_string(), items, filter: filter.to_string(), selected: 0 }
    }

    pub fn visible(&self) -> Vec<&PickItem> {
        let needle = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|item| {
                needle.is_empty()
                    || item.label.to_lowercase().contains(&needle)
                    || item.hint.to_lowercase().contains(&needle)
                    || item.value.to_lowercase().contains(&needle)
            })
            .collect()
    }

    fn move_selection(&mut self, step: isize) {
        let count = self.visible().len();
        if count > 0 {
            self.selected = (self.selected as isize + step).clamp(0, count as isize - 1) as usize;
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Enter => {
                let value = self.visible().get(self.selected).map(|item| item.value.clone());
                if let Some(value) = value {
                    return Action::Pick(self.kind, value);
                }
            }
            KeyCode::Esc => return Action::Close,
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(c) if !is_ctrl(&key) => {
                self.filter.push(c);
                self.selected = 0;
            }
            _ => {}
        }
        Action::None
    }
}

pub fn bbcode_items() -> Vec<PickItem> {
    let tags = [
        ("Bold", "[b]{}[/b]"),
        ("Italic", "[i]{}[/i]"),
        ("Underline", "[u]{}[/u]"),
        ("Strikethrough", "[s]{}[/s]"),
        ("Subscript", "[sub]{}[/sub]"),
        ("Superscript", "[sup]{}[/sup]"),
        ("Big", "[big]{}[/big]"),
        ("Small", "[small]{}[/small]"),
        ("Heading", "[heading]{}[/heading]"),
        ("Spoiler", "[spoiler]{}[/spoiler]"),
        ("Quote", "[quote]{}[/quote]"),
        ("No parse", "[noparse]{}[/noparse]"),
        ("Horizontal rule", "[hr]"),
        ("User link", "[user]{}[/user]"),
        ("Icon", "[icon]{}[/icon]"),
        ("Eicon", "[eicon]{}[/eicon]"),
        ("Link", "[url={}][/url]"),
        ("Session link", "[session=Title]{}[/session]"),
        ("Channel link", "[channel]{}[/channel]"),
    ];
    let mut items: Vec<PickItem> =
        tags.iter().map(|(label, template)| PickItem { label: label.to_string(), value: template.to_string(), hint: template.replace("{}", "") }).collect();
    for name in BBCODE_COLOR_NAMES {
        items.push(PickItem {
            label: format!("Color {name}"),
            value: format!("[color={name}]{{}}[/color]"),
            hint: format!("[color={name}]"),
        });
    }
    items
}

pub fn emoji_items() -> Vec<PickItem> {
    let mut items = Vec::new();
    let ranges = [0x2600u32..0x27C0, 0x1F300..0x1F650, 0x1F900..0x1FAFF];
    for range in ranges {
        for code in range {
            let Some(c) = char::from_u32(code) else { continue };
            let Some(name) = unicode_names2::name(c) else { continue };
            let name = name.to_string().to_lowercase();
            items.push(PickItem { label: format!("{c}  {name}"), value: c.to_string(), hint: name });
        }
    }
    items
}

// ------------------------------------------------------------------ edit

pub struct EditScreen {
    pub title: String,
    pub input: InputBox,
    pub max: usize,
}

impl EditScreen {
    pub fn new(title: &str, text: &str, max: usize) -> EditScreen {
        let mut input = InputBox::new();
        input.history_enabled = false;
        input.set_text(text);
        EditScreen { title: title.to_string(), input, max }
    }

    pub fn insert(&mut self, text: &str) {
        let room = self.max.saturating_sub(self.input.char_count());
        let text: String = text.chars().filter(|c| !c.is_control()).take(room).collect();
        self.input.insert_str(&text);
    }

    fn newline(&mut self) {
        if self.input.char_count() < self.max {
            self.input.newline();
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT) => self.newline(),
            KeyCode::Char('j') if is_ctrl(&key) => self.newline(),
            KeyCode::Enter => return Action::Edit(self.input.text().to_string()),
            KeyCode::Esc => return Action::Close,
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Up => {
                self.input.move_up();
            }
            KeyCode::Down => {
                self.input.move_down();
            }
            KeyCode::Char('u') if is_ctrl(&key) => self.input.set_text(""),
            KeyCode::Char('a') if is_ctrl(&key) => self.input.home(),
            KeyCode::Char('e') if is_ctrl(&key) => self.input.end(),
            KeyCode::Char(c) if !is_ctrl(&key) => self.insert(&c.to_string()),
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_screen_edits_in_the_middle_and_respects_the_limit() {
        let mut edit = EditScreen::new("Status", "hallo welt", 12);
        for _ in 0..5 {
            edit.on_key(KeyEvent::from(KeyCode::Left));
        }
        edit.on_key(KeyEvent::from(KeyCode::Char('!')));
        assert_eq!(edit.input.text(), "hallo! welt");
        edit.on_key(KeyEvent::from(KeyCode::Delete));
        assert_eq!(edit.input.text(), "hallo!welt");
        edit.insert("abcdef");
        assert_eq!(edit.input.char_count(), 12);
        edit.on_key(KeyEvent::from(KeyCode::Up));
        edit.on_key(KeyEvent::from(KeyCode::Down));
        edit.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(edit.input.text().ends_with('\n') || edit.input.char_count() == 12);
        assert!(matches!(edit.on_key(KeyEvent::from(KeyCode::Enter)), Action::Edit(_)));
    }
}
