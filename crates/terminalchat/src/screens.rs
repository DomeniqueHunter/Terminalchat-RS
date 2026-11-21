//! Modal screens: login, character choice, profile, info, pickers, a one-line editor and the
//! settings page.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::bbcode::Seg;
use crate::colors::{self, Kind, Parts, MODIFIERS, PRESETS};
use crate::config::Config;
use crate::input::InputBox;
use crate::settings::{self, Field, SETTINGS};
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
    Setting(String, String),
    ResetSetting(String),
    Color(String, String),
    ResetColor(String),
    /// Everything of one tab of the settings page back to default.
    ResetTab(Tab),
    /// Opens or copies a link, e.g. "o" on a profile.
    OpenLink(String),
}

pub enum Screen {
    Login(LoginScreen),
    Character(CharacterScreen),
    Profile(ProfileScreen),
    Info(InfoScreen),
    Picker(PickerScreen),
    Edit(EditScreen),
    Settings(SettingsScreen),
    Notes(Box<crate::notes::NotesScreen>),
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
            Screen::Settings(screen) => screen.on_key(key),
            Screen::Notes(screen) => screen.on_key(key),
        }
    }

    /// Scrolls by mouse wheel (positive: down).
    pub fn scroll(&mut self, rows: isize) {
        match self {
            Screen::Profile(screen) => {
                let scroll = screen.scroll_mut();
                *scroll = scroll.saturating_add_signed(rows);
            }
            Screen::Info(screen) => screen.scroll = screen.scroll.saturating_add_signed(rows),
            Screen::Picker(screen) => screen.move_selection(rows),
            Screen::Character(screen) => screen.move_selection(rows),
            Screen::Settings(screen) if screen.edit.is_none() => screen.move_selection(rows),
            Screen::Notes(screen) => screen.scroll(rows),
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileTab {
    Info,
    Description,
    Kinks,
    Images,
}

pub const PROFILE_TABS: [ProfileTab; 4] = [ProfileTab::Info, ProfileTab::Description, ProfileTab::Kinks, ProfileTab::Images];

/// A profile like in the Python version: a header with avatar, name, status and link, below it
/// the tabs Info, Description, Kinks and Images, each scrolled on its own.
pub struct ProfileScreen {
    pub character: String,
    pub result: Option<Result<fchat::Profile, String>>,
    pub tab: ProfileTab,
    pub scroll: [usize; 4],
    /// Click areas of the tabs, filled when drawn.
    pub tab_hits: Vec<(Rect, ProfileTab)>,
}

impl ProfileScreen {
    pub fn new(character: &str) -> ProfileScreen {
        ProfileScreen { character: character.to_string(), result: None, tab: ProfileTab::Info, scroll: [0; 4], tab_hits: Vec::new() }
    }

    pub fn scroll_mut(&mut self) -> &mut usize {
        &mut self.scroll[self.tab as usize]
    }

    fn step_tab(&mut self, step: isize) {
        let position = PROFILE_TABS.iter().position(|tab| *tab == self.tab).unwrap_or(0) as isize;
        self.tab = PROFILE_TABS[(position + step).rem_euclid(PROFILE_TABS.len() as isize) as usize];
    }

    /// A click on a tab switches to it.
    pub fn click_tab(&mut self, column: u16, row: u16) -> bool {
        let hit = self.tab_hits.iter().find(|(area, _)| column >= area.x && column < area.x + area.width && row >= area.y && row < area.y + area.height);
        match hit {
            Some((_, tab)) => {
                self.tab = *tab;
                true
            }
            None => false,
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        let scroll = self.scroll_mut();
        match key.code {
            KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::Down => *scroll += 1,
            KeyCode::PageUp => *scroll = scroll.saturating_sub(15),
            KeyCode::PageDown => *scroll += 15,
            KeyCode::Home => *scroll = 0,
            KeyCode::End => *scroll = usize::MAX / 2,
            KeyCode::Left | KeyCode::BackTab => self.step_tab(-1),
            KeyCode::Right | KeyCode::Tab => self.step_tab(1),
            KeyCode::Char('o') => return Action::OpenLink(crate::bbcode::profile_url(&self.character)),
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

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    General,
    Colors,
}

pub enum SettingsEdit {
    Text(InputBox),
    Color(ColorEditor),
}

/// The parts of the color editor, walked with tab / up / down.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorPart {
    Text,
    Color,
    Background,
    Modifiers,
}

/// What a click on the settings page hits; the drawing code fills the list each frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Tab(Tab),
    Reset,
    Row(usize),
    Part(ColorPart),
    Swatch(ColorPart, usize),
    Modifier(usize),
}

pub struct ColorEditor {
    pub path: String,
    pub kind: Kind,
    pub input: InputBox,
    pub parts: Parts,
    pub part: ColorPart,
    /// 0 is "none", then PRESETS from 1.
    pub color: usize,
    pub background: usize,
    pub modifier: usize,
}

impl ColorEditor {
    fn new(path: &str, kind: Kind, spec: &str) -> ColorEditor {
        let mut input = InputBox::new();
        input.history_enabled = false;
        input.set_text(spec);
        let parts = Parts::parse(spec, kind).unwrap_or_default();
        let color = preset_index(parts.color.as_deref());
        let background = preset_index(parts.background.as_deref());
        ColorEditor { path: path.to_string(), kind, input, parts, part: ColorPart::Text, color, background, modifier: 0 }
    }

    /// The parts this kind of entry has: a background is only a color.
    pub fn part_list(&self) -> &'static [ColorPart] {
        match self.kind {
            Kind::Background => &[ColorPart::Text, ColorPart::Color],
            Kind::Style => &[ColorPart::Text, ColorPart::Color, ColorPart::Background, ColorPart::Modifiers],
        }
    }

    fn step_part(&mut self, step: isize) {
        let list = self.part_list();
        let position = list.iter().position(|part| *part == self.part).unwrap_or(0) as isize;
        self.part = list[(position + step).rem_euclid(list.len() as isize) as usize];
    }

    /// The text was typed: the picker follows when it can be read.
    fn text_changed(&mut self) {
        if let Some(parts) = Parts::parse(self.input.text(), self.kind) {
            self.color = preset_index(parts.color.as_deref());
            self.background = preset_index(parts.background.as_deref());
            self.parts = parts;
        }
    }

    /// The picker changed: the text follows.
    fn parts_changed(&mut self) {
        self.input.set_text(&self.parts.to_spec(self.kind));
    }

    /// Picks a swatch of the current picker part: 0 is "none".
    fn choose(&mut self, index: usize) {
        let index = index.min(PRESETS.len());
        let chosen = index.checked_sub(1).map(|preset| PRESETS[preset].to_string());
        if self.part == ColorPart::Color {
            self.color = index;
            self.parts.color = chosen;
        } else if self.part == ColorPart::Background {
            self.background = index;
            self.parts.background = chosen;
        } else {
            return;
        }
        self.parts_changed();
    }

    fn toggle_modifier(&mut self, index: usize) {
        self.modifier = index.min(MODIFIERS.len() - 1);
        self.parts.toggle(MODIFIERS[self.modifier]);
        self.parts_changed();
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Enter => return Action::Color(self.path.clone(), self.input.text().to_string()),
            KeyCode::Tab | KeyCode::Down => self.step_part(1),
            KeyCode::BackTab | KeyCode::Up => self.step_part(-1),
            _ => match self.part {
                ColorPart::Text => {
                    match key.code {
                        KeyCode::Left => self.input.left(),
                        KeyCode::Right => self.input.right(),
                        KeyCode::Home => self.input.home(),
                        KeyCode::End => self.input.end(),
                        KeyCode::Backspace => self.input.backspace(),
                        KeyCode::Delete => self.input.delete(),
                        KeyCode::Char('u') if is_ctrl(&key) => self.input.set_text(""),
                        KeyCode::Char(c) if !is_ctrl(&key) => self.input.insert_char(c),
                        _ => return Action::None,
                    }
                    self.text_changed();
                }
                ColorPart::Color | ColorPart::Background => {
                    let step: isize = match key.code {
                        KeyCode::Left => -1,
                        KeyCode::Right => 1,
                        KeyCode::PageUp | KeyCode::Home => -(PRESETS.len() as isize),
                        KeyCode::PageDown | KeyCode::End => PRESETS.len() as isize,
                        _ => return Action::None,
                    };
                    let current = if self.part == ColorPart::Color { self.color } else { self.background };
                    self.choose((current as isize + step).clamp(0, PRESETS.len() as isize) as usize);
                }
                ColorPart::Modifiers => match key.code {
                    KeyCode::Left => self.modifier = self.modifier.saturating_sub(1),
                    KeyCode::Right => self.modifier = (self.modifier + 1).min(MODIFIERS.len() - 1),
                    KeyCode::Char(' ') => self.toggle_modifier(self.modifier),
                    _ => {}
                },
            },
        }
        Action::None
    }
}

/// Position in the picker: 0 is "none", the presets follow.
fn preset_index(color: Option<&str>) -> usize {
    color.and_then(|color| PRESETS.iter().position(|preset| preset.eq_ignore_ascii_case(color))).map_or(0, |position| position + 1)
}

/// The settings page: general entries of config.json and all colors.
pub struct SettingsScreen {
    pub tab: Tab,
    /// Selected entry per tab.
    pub selected: [usize; 2],
    pub edit: Option<SettingsEdit>,
    /// Text and whether it is an error.
    pub message: Option<(String, bool)>,
    /// The general values as shown, refreshed after each change.
    pub values: Vec<String>,
    /// Click areas of the last drawing.
    pub hits: Vec<(Rect, Hit)>,
    /// The reset of the current tab waits for a yes.
    pub confirm_reset: bool,
}

impl SettingsScreen {
    pub fn new(config: &Config) -> SettingsScreen {
        let mut screen = SettingsScreen { tab: Tab::General, selected: [0, 0], edit: None, message: None, values: Vec::new(), hits: Vec::new(), confirm_reset: false };
        screen.refresh(config);
        screen
    }

    pub fn refresh(&mut self, config: &Config) {
        self.values = SETTINGS.iter().map(|setting| settings::value(config, setting.key)).collect();
    }

    fn count(&self) -> usize {
        match self.tab {
            Tab::General => SETTINGS.len(),
            Tab::Colors => colors::entries().len(),
        }
    }

    pub fn current(&self) -> usize {
        self.selected[self.tab as usize]
    }

    fn move_selection(&mut self, step: isize) {
        let count = self.count() as isize;
        let index = &mut self.selected[self.tab as usize];
        *index = (*index as isize + step).clamp(0, count - 1) as usize;
    }

    /// Result of a change from the app: an error keeps the editor open.
    pub fn done(&mut self, result: Result<String, String>, config: &Config) {
        match result {
            Ok(message) => {
                self.edit = None;
                self.message = (!message.is_empty()).then_some((message, false));
            }
            Err(error) => self.message = Some((error, true)),
        }
        self.refresh(config);
    }

    fn ask_reset(&mut self) {
        self.confirm_reset = true;
        let what = if self.tab == Tab::General { "general settings" } else { "colors" };
        self.message = Some((format!("Reset all {what} to default? y: yes, any other key: no"), true));
    }

    fn on_key(&mut self, key: KeyEvent) -> Action {
        if self.confirm_reset {
            self.confirm_reset = false;
            self.message = None;
            return if key.code == KeyCode::Char('y') { Action::ResetTab(self.tab) } else { Action::None };
        }
        if let Some(edit) = &mut self.edit {
            let action = match edit {
                SettingsEdit::Color(editor) => editor.on_key(key),
                SettingsEdit::Text(input) => match key.code {
                    KeyCode::Esc => Action::Close,
                    KeyCode::Enter => {
                        let key = SETTINGS[self.selected[0]].key;
                        Action::Setting(key.to_string(), input.text().to_string())
                    }
                    KeyCode::Left => {
                        input.left();
                        Action::None
                    }
                    KeyCode::Right => {
                        input.right();
                        Action::None
                    }
                    KeyCode::Home => {
                        input.home();
                        Action::None
                    }
                    KeyCode::End => {
                        input.end();
                        Action::None
                    }
                    KeyCode::Backspace => {
                        input.backspace();
                        Action::None
                    }
                    KeyCode::Delete => {
                        input.delete();
                        Action::None
                    }
                    KeyCode::Char('u') if is_ctrl(&key) => {
                        input.set_text("");
                        Action::None
                    }
                    KeyCode::Char(c) if !is_ctrl(&key) => {
                        input.insert_char(c);
                        Action::None
                    }
                    _ => Action::None,
                },
            };
            if matches!(action, Action::Close) {
                self.edit = None;
                self.message = None;
                return Action::None;
            }
            return action;
        }
        self.message = None;
        match key.code {
            KeyCode::Esc => return Action::Close,
            KeyCode::Tab | KeyCode::BackTab => self.tab = if self.tab == Tab::General { Tab::Colors } else { Tab::General },
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::Home => self.move_selection(-(self.count() as isize)),
            KeyCode::End => self.move_selection(self.count() as isize),
            KeyCode::Char('R') => self.ask_reset(),
            KeyCode::Char('r') => {
                return match self.tab {
                    Tab::General => Action::ResetSetting(SETTINGS[self.current()].key.to_string()),
                    Tab::Colors => Action::ResetColor(colors::entries()[self.current()].path.clone()),
                };
            }
            KeyCode::Enter | KeyCode::Char(' ') => return self.activate(),
            _ => {}
        }
        Action::None
    }

    /// Enter on an entry: switch, cycle or open the editor.
    fn activate(&mut self) -> Action {
        match self.tab {
            Tab::General => {
                let index = self.current();
                let setting = &SETTINGS[index];
                let current = self.values.get(index).cloned().unwrap_or_default();
                match setting.field {
                    Field::Bool => return Action::Setting(setting.key.to_string(), (current != "true").to_string()),
                    Field::Choice(choices) => {
                        let position = choices.iter().position(|choice| *choice == current).map_or(0, |position| position + 1);
                        return Action::Setting(setting.key.to_string(), choices[position % choices.len()].to_string());
                    }
                    _ => {
                        let mut input = InputBox::new();
                        input.history_enabled = false;
                        input.set_text(&current);
                        self.edit = Some(SettingsEdit::Text(input));
                    }
                }
            }
            Tab::Colors => {
                let entry = &colors::entries()[self.current()];
                let spec = colors::palette().spec(&entry.path).to_string();
                self.edit = Some(SettingsEdit::Color(ColorEditor::new(&entry.path, entry.kind, &spec)));
            }
        }
        Action::None
    }

    /// A left click: tabs switch, a row is chosen and a click on the chosen row changes it, in the
    /// color editor parts, swatches and check boxes react.
    pub fn click(&mut self, column: u16, row: u16) -> Action {
        let inside = |rect: &Rect| column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height;
        let Some(hit) = self.hits.iter().find(|(rect, _)| inside(rect)).map(|(_, hit)| *hit) else { return Action::None };
        if hit == Hit::Reset {
            if self.confirm_reset {
                self.confirm_reset = false;
                self.message = None;
                return Action::ResetTab(self.tab);
            }
            self.edit = None;
            self.ask_reset();
            return Action::None;
        }
        self.confirm_reset = false;
        self.message = None;
        match hit {
            Hit::Reset => {}
            Hit::Tab(tab) => {
                if self.tab != tab {
                    self.edit = None;
                    self.tab = tab;
                }
            }
            Hit::Row(index) if index == self.current() => {
                if self.edit.is_none() {
                    return self.activate();
                }
            }
            Hit::Row(index) => {
                self.edit = None;
                self.selected[self.tab as usize] = index;
            }
            Hit::Part(part) => {
                if let Some(SettingsEdit::Color(editor)) = &mut self.edit {
                    editor.part = part;
                }
            }
            Hit::Swatch(part, index) => {
                if let Some(SettingsEdit::Color(editor)) = &mut self.edit {
                    editor.part = part;
                    editor.choose(index);
                }
            }
            Hit::Modifier(index) => {
                if let Some(SettingsEdit::Color(editor)) = &mut self.edit {
                    editor.part = ColorPart::Modifiers;
                    editor.toggle_modifier(index);
                }
            }
        }
        Action::None
    }

    /// Pasted text goes into the field being edited.
    pub fn paste(&mut self, text: &str) {
        match &mut self.edit {
            Some(SettingsEdit::Text(input)) => input.insert_str(text),
            Some(SettingsEdit::Color(editor)) if editor.part == ColorPart::Text => {
                editor.input.insert_str(text);
                editor.text_changed();
            }
            _ => {}
        }
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

    fn press(screen: &mut SettingsScreen, code: KeyCode) -> Action {
        screen.on_key(KeyEvent::from(code))
    }

    fn select(screen: &mut SettingsScreen, key: &str) {
        screen.selected[0] = SETTINGS.iter().position(|setting| setting.key == key).unwrap();
    }

    #[test]
    fn switches_and_choices_need_no_editor() {
        let mut screen = SettingsScreen::new(&Config::default());
        select(&mut screen, "show_sidebar");
        assert!(matches!(press(&mut screen, KeyCode::Enter), Action::Setting(key, value) if key == "show_sidebar" && value == "true"));
        select(&mut screen, "eicons");
        assert!(matches!(press(&mut screen, KeyCode::Char(' ')), Action::Setting(key, value) if key == "eicons" && value == "off"));
        select(&mut screen, "protocol");
        assert!(matches!(press(&mut screen, KeyCode::Char('r')), Action::ResetSetting(key) if key == "protocol"));
        assert!(screen.edit.is_none());
    }

    #[test]
    fn text_is_edited_in_the_row_and_an_error_keeps_the_editor() {
        let config = Config::default();
        let mut screen = SettingsScreen::new(&config);
        select(&mut screen, "eicon_height");
        press(&mut screen, KeyCode::Enter);
        press(&mut screen, KeyCode::Backspace);
        press(&mut screen, KeyCode::Char('9'));
        let action = press(&mut screen, KeyCode::Enter);
        assert!(matches!(action, Action::Setting(key, value) if key == "eicon_height" && value == "9"));
        screen.done(Err("a whole number from 1 to 20".into()), &config);
        assert!(screen.edit.is_some());
        assert_eq!(screen.message.as_ref().map(|(_, error)| *error), Some(true));
        screen.done(Ok("eicon_height saved".into()), &config);
        assert!(screen.edit.is_none());
        press(&mut screen, KeyCode::Enter);
        press(&mut screen, KeyCode::Esc);
        assert!(screen.edit.is_none(), "esc only closes the editor");
        assert!(matches!(press(&mut screen, KeyCode::Esc), Action::Close));
    }

    #[test]
    fn the_color_picker_and_the_text_follow_each_other() {
        let mut screen = SettingsScreen::new(&Config::default());
        press(&mut screen, KeyCode::Tab);
        screen.selected[1] = colors::entries().iter().position(|entry| entry.path == "url").unwrap();
        press(&mut screen, KeyCode::Enter);
        let editor = |screen: &SettingsScreen| match &screen.edit {
            Some(SettingsEdit::Color(editor)) => (editor.input.text().to_string(), editor.part, editor.color),
            _ => panic!("no color editor"),
        };
        assert_eq!(editor(&screen).0, "underline #5f87ff");
        assert_eq!(editor(&screen).2, PRESETS.iter().position(|preset| *preset == "#5f87ff").unwrap() + 1);
        press(&mut screen, KeyCode::Down);
        press(&mut screen, KeyCode::Home);
        assert_eq!(editor(&screen).0, "underline");
        press(&mut screen, KeyCode::End);
        assert_eq!(editor(&screen).0, "underline #ff87d7");
        press(&mut screen, KeyCode::Down);
        press(&mut screen, KeyCode::Right);
        assert_eq!(editor(&screen).0, "underline #ff87d7 on #000000");
        press(&mut screen, KeyCode::Down);
        press(&mut screen, KeyCode::Char(' '));
        assert_eq!(editor(&screen).0, "bold underline #ff87d7 on #000000");
        press(&mut screen, KeyCode::Down);
        assert_eq!(editor(&screen).1, ColorPart::Text);
        press(&mut screen, KeyCode::End);
        for c in " #ff0000".chars() {
            press(&mut screen, KeyCode::Char(c));
        }
        assert_eq!(editor(&screen).2, 0, "a color outside the presets marks none");
        let action = press(&mut screen, KeyCode::Enter);
        assert!(matches!(action, Action::Color(path, spec) if path == "url" && spec == "bold underline #ff87d7 on #000000 #ff0000"));
    }

    #[test]
    fn a_tab_is_reset_only_after_a_yes() {
        let mut screen = SettingsScreen::new(&Config::default());
        press(&mut screen, KeyCode::Tab);
        press(&mut screen, KeyCode::Char('R'));
        assert!(screen.confirm_reset);
        assert!(matches!(press(&mut screen, KeyCode::Char('n')), Action::None));
        assert!(!screen.confirm_reset);
        press(&mut screen, KeyCode::Char('R'));
        assert!(matches!(press(&mut screen, KeyCode::Char('y')), Action::ResetTab(Tab::Colors)));
    }

    #[test]
    fn profile_tabs_switch_and_scroll_on_their_own() {
        let mut profile = ProfileScreen::new("Somebody");
        profile.on_key(KeyEvent::from(KeyCode::Down));
        profile.on_key(KeyEvent::from(KeyCode::Down));
        profile.on_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(profile.tab, ProfileTab::Description);
        assert_eq!(*profile.scroll_mut(), 0);
        profile.on_key(KeyEvent::from(KeyCode::BackTab));
        profile.on_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(profile.tab, ProfileTab::Images);
        profile.on_key(KeyEvent::from(KeyCode::Right));
        assert_eq!((profile.tab, profile.scroll[0]), (ProfileTab::Info, 2));
        assert!(matches!(profile.on_key(KeyEvent::from(KeyCode::Char('o'))), Action::OpenLink(url) if url.ends_with("/c/Somebody")));
    }
}
