//! Keyboard, mouse and paste handling.

use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};


use crate::app::{App, Focus, Selection, SidebarTab, TicketPurpose};
use crate::commands::{matching_commands, resolve_alias, NAME_COMMANDS};
use crate::screens::{Action, PickerKind, Screen};
use crate::store::CHANNEL;

const WHEEL_ROWS: isize = 3;

fn contains(rect: &ratatui::layout::Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

pub fn handle(app: &mut App, event: Event) {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => on_key(app, key),
        Event::Mouse(mouse) => on_mouse(app, mouse),
        Event::Paste(text) => paste(app, &text),
        _ => {}
    }
    app.flush_notes();
    app.flush_friends();
}

fn paste(app: &mut App, text: &str) {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    match app.screens.last_mut() {
        Some(Screen::Picker(picker)) => picker.filter.push_str(&text.replace('\n', " ")),
        Some(Screen::Edit(edit)) => edit.insert(&text.replace('\n', " ")),
        Some(Screen::Settings(settings)) => settings.paste(&text.replace('\n', " ")),
        Some(Screen::Notes(notes)) => notes.paste(&text),
        Some(Screen::Friends(friends)) => friends.paste(&text),
        Some(Screen::Login(login)) => match login.focus {
            0 => login.account.push_str(text.trim()),
            1 => login.password.push_str(text.trim()),
            _ => {}
        },
        Some(_) => {}
        None => {
            app.focus = Focus::Input;
            app.input.insert_str(&text);
        }
    }
}

fn next_focus(app: &App, step: isize) -> Focus {
    let mut order = vec![Focus::Input];
    if !app.regions.chats.is_empty() {
        order.push(Focus::List);
    }
    order.push(Focus::Chat);
    if !app.regions.sidebar_tabs.is_empty() {
        order.push(Focus::Sidebar);
    }
    let current = order.iter().position(|focus| *focus == app.focus).unwrap_or(0) as isize;
    order[(current + step).rem_euclid(order.len() as isize) as usize]
}

/// Keys while a panel (not the input) has the focus. Returns false if the key is not handled here.
fn panel_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return false;
    }
    match key.code {
        KeyCode::Tab => app.focus = next_focus(app, 1),
        KeyCode::BackTab => app.focus = next_focus(app, -1),
        KeyCode::Esc => app.focus = Focus::Input,
        KeyCode::PageUp => scroll_chat(app, -1),
        KeyCode::PageDown => scroll_chat(app, 1),
        KeyCode::Char(c) => {
            app.focus = Focus::Input;
            app.input.insert_char(c);
        }
        KeyCode::Up | KeyCode::Down => {
            let direction: isize = if key.code == KeyCode::Up { -1 } else { 1 };
            match app.focus {
                Focus::List => app.select_relative_chat(direction),
                Focus::Chat => {
                    app.pager_op(|pager, src| pager.scroll_by(src, -direction));
                }
                _ => {
                    let last = app.regions.names.len().saturating_sub(1);
                    app.sidebar_cursor = (app.sidebar_cursor as isize + direction).clamp(0, last as isize) as usize;
                }
            }
        }
        KeyCode::Home if app.focus == Focus::Chat => {
            app.pager_op(|pager, src| pager.jump_to_start(src));
        }
        KeyCode::End if app.focus == Focus::Chat => {
            app.pager_op(|pager, src| pager.jump_to_end(src));
        }
        KeyCode::Left | KeyCode::Right if app.focus == Focus::Sidebar => {
            app.sidebar_tab = if app.sidebar_tab == SidebarTab::Friends { SidebarTab::Channel } else { SidebarTab::Friends };
            app.sidebar_cursor = 0;
        }
        KeyCode::Enter => {
            if app.focus == Focus::Sidebar {
                if let Some((_, name)) = app.regions.names.get(app.sidebar_cursor).cloned() {
                    app.open_private(&name);
                }
            }
            app.focus = Focus::Input;
        }
        _ => {}
    }
    true
}

fn on_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v') {
        let text = app.clipboard.clone();
        paste(app, &text);
        return;
    }
    if let Some(screen) = app.screens.last_mut() {
        let action = screen.on_key(key);
        apply_action(app, action);
        return;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let selected = app.selected_text();
    app.selection = None;
    if app.focus != Focus::Input && panel_key(app, key) {
        return;
    }
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char(c) if ctrl => {
            if c.eq_ignore_ascii_case(&'c') && !selected.is_empty() {
                app.copy_to_clipboard(&selected);
            } else {
                ctrl_key(app, c.to_ascii_lowercase());
            }
        }
        KeyCode::Char(c) if alt => {
            let template = match c.to_ascii_lowercase() {
                'b' => "[b]{}[/b]",
                'i' => "[i]{}[/i]",
                'u' => "[u]{}[/u]",
                's' => "[s]{}[/s]",
                'p' => "[spoiler]{}[/spoiler]",
                'q' => "[quote]{}[/quote]",
                _ => return,
            };
            app.input.insert_template(template);
        }
        KeyCode::Char(c) => app.input.insert_char(c),
        KeyCode::Enter if alt || shift || ctrl => app.input.newline(),
        KeyCode::Enter => app.submit(),
        KeyCode::Backspace => app.input.backspace(),
        KeyCode::Delete => app.input.delete(),
        KeyCode::Left => app.input.left(),
        KeyCode::Right => app.input.right(),
        KeyCode::Home => app.input.home(),
        KeyCode::End => app.input.end(),
        KeyCode::Up if alt => app.select_relative_chat(-1),
        KeyCode::Down if alt => app.select_relative_chat(1),
        KeyCode::Up => app.input.history_prev(),
        KeyCode::Down => app.input.history_next(),
        KeyCode::PageUp if ctrl => {
            app.pager_op(|pager, src| pager.jump_to_start(src));
        }
        KeyCode::PageDown if ctrl => {
            app.pager_op(|pager, src| pager.jump_to_end(src));
        }
        KeyCode::PageUp => scroll_chat(app, -1),
        KeyCode::PageDown => scroll_chat(app, 1),
        KeyCode::Tab | KeyCode::BackTab => {
            let step = if key.code == KeyCode::Tab { 1 } else { -1 };
            if app.input.text().trim().is_empty() {
                app.focus = next_focus(app, step);
            } else {
                complete(app, step);
            }
        }
        KeyCode::F(1) => app.open_picker(PickerKind::Commands, ""),
        _ => {}
    }
}

fn ctrl_key(app: &mut App, c: char) {
    match c {
        'q' => app.should_quit = true,
        'c' => app.input.clear_with_undo(),
        'z' => app.input.undo_clear(),
        'a' => app.toggle_ad_mode(),
        'e' => app.open_picker(PickerKind::Emoji, ""),
        'h' => app.open_picker(PickerKind::Commands, ""),
        'b' => app.open_picker(PickerKind::Bbcode, ""),
        'o' => app.open_channel_list(),
        'l' => app.close_chat(),
        'f' => app.show_sidebar = !app.show_sidebar,
        't' => app.minimize_requested = true,
        'p' => app.run_command("/profile"),
        'n' => app.show_notes(),
        'j' => app.input.newline(),
        'v' => {
            let text = app.clipboard.clone();
            paste(app, &text);
        }
        _ => {}
    }
}

fn scroll_chat(app: &mut App, direction: isize) {
    let page = (app.pager.height as isize - 2).max(1);
    app.pager_op(|pager, src| pager.scroll_by(src, -direction * page));
}

fn complete(app: &mut App, step: isize) {
    if app.input.is_command_word() {
        let prefix = app.input.text()[1..].to_string();
        let found = matching_commands(&prefix);
        let names: Vec<&str> = found.iter().map(|entry| entry.0).collect();
        app.input.complete_command(&names);
        return;
    }
    let code = app.current_chat().filter(|chat| chat.kind == CHANNEL).map(|chat| chat.key.clone());
    let App { input, presence, .. } = app;
    let in_name_command = input.in_name_command(NAME_COMMANDS, resolve_alias);
    let note = input.complete_name(step, in_name_command, |prefix| presence.complete_names(prefix, code.as_deref()));
    if let Some(note) = note {
        app.notify_titled("", note, crate::app::Severity::Information, 3);
    }
}

fn apply_action(app: &mut App, action: Action) {
    match action {
        Action::None => {}
        Action::Close => {
            let closed = app.screens.pop();
            if matches!(closed, Some(Screen::Character(_))) {
                app.on_character_chosen(None);
            }
        }
        Action::Login { account, password, remember } => {
            app.spawn_ticket(TicketPurpose::Login { account: account.clone(), password: password.clone(), remember }, account, password);
        }
        Action::Character(name) => {
            app.screens.pop();
            app.on_character_chosen(Some(name));
        }
        Action::Pick(kind, value) => {
            app.screens.pop();
            match kind {
                PickerKind::Emoji => app.input.insert_str(&value),
                PickerKind::Bbcode => app.input.insert_template(&value),
                PickerKind::Channels => app.join(&value, true),
                PickerKind::Commands => app.input.set_text(&value),
            }
        }
        Action::Edit(text) => {
            app.screens.pop();
            let status = app.own_status.status.clone();
            if text != app.own_status.message {
                app.apply_status(&status, text.trim());
            }
        }
        Action::Setting(key, text) => {
            let result = app.change_setting(&key, &text);
            settings_done(app, result);
        }
        Action::ResetSetting(key) => {
            let result = app.reset_setting(&key);
            settings_done(app, result);
        }
        Action::Color(path, spec) => {
            let result = app.change_color(&path, &spec);
            settings_done(app, result);
        }
        Action::ResetColor(path) => {
            let result = app.reset_color(&path);
            settings_done(app, result);
        }
        Action::ResetTab(tab) => {
            let result = app.reset_tab(tab);
            settings_done(app, result);
        }
        Action::OpenLink(url) => app.open_link(&url, None),
        Action::Private(name) => {
            app.screens.clear();
            app.open_private(&name);
        }
        Action::Profile(name) => app.show_profile(&name),
        Action::Bookmark(name) => app.toggle_bookmark(&name),
        Action::FriendRequest(name) => app.friend_request(&name),
    }
}

fn settings_done(app: &mut App, result: Result<String, String>) {
    let App { screens, config, .. } = app;
    if let Some(Screen::Settings(settings)) = screens.last_mut() {
        settings.done(result, config);
    }
}

fn on_mouse(app: &mut App, mouse: MouseEvent) {
    let (column, row) = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
            app.selection = None;
            let up = mouse.kind == MouseEventKind::ScrollUp;
            if let Some(screen) = app.screens.last_mut() {
                screen.scroll(if up { -WHEEL_ROWS } else { WHEEL_ROWS });
            } else {
                app.pager_op(|pager, src| pager.scroll_by(src, if up { WHEEL_ROWS } else { -WHEEL_ROWS }));
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            app.selection = None;
            app.press = None;
            if app.screens.is_empty() && contains(&app.regions.chat, column, row) {
                app.press = Some((column, row));
            } else {
                on_click(app, column, row);
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some(start) = app.press {
                let area = app.regions.chat;
                let column = column.clamp(area.x, area.x + area.width.saturating_sub(1));
                let row = row.clamp(area.y, area.y + area.height.saturating_sub(1));
                app.selection = ((column, row) != start).then_some(Selection { start, end: (column, row) });
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some((press_column, press_row)) = app.press.take() {
                if app.selection.is_none() {
                    on_click(app, press_column, press_row);
                } else {
                    let text = app.selected_text();
                    if !text.is_empty() {
                        app.copy_to_clipboard(&text);
                        let count = text.chars().count();
                        app.notify_titled("", format!("Copied {count} characters"), crate::app::Severity::Information, 2);
                    }
                }
            }
        }
        _ => {}
    }
}

fn on_click(app: &mut App, column: u16, row: u16) {
    if let Some(Screen::Settings(settings)) = app.screens.last_mut() {
        let action = settings.click(column, row);
        apply_action(app, action);
        return;
    }
    if let Some(Screen::Notes(notes)) = app.screens.last_mut() {
        notes.click(column, row);
    }
    if let Some(Screen::Friends(friends)) = app.screens.last_mut() {
        let action = friends.click(column, row);
        apply_action(app, action);
        return;
    }
    if let Some(Screen::Profile(profile)) = app.screens.last_mut() {
        if profile.click_tab(column, row) {
            return;
        }
    }
    if !app.screens.is_empty() {
        let link = app.regions.modal_links.iter().find(|(rect, _)| contains(rect, column, row)).map(|(_, link)| link.clone());
        if let Some(link) = link {
            let internal = link.starts_with(crate::bbcode::USER_LINK) || link.starts_with(crate::bbcode::CHANNEL_LINK);
            if internal && matches!(app.screens.last(), Some(Screen::Profile(_))) {
                app.screens.pop();
            }
            app.open_link(&link, None);
        }
        return;
    }
    if let Some((_, key)) = app.regions.footer.iter().find(|(rect, _)| contains(rect, column, row)).cloned() {
        ctrl_key(app, key);
        return;
    }
    if let Some((_, mode)) = app.regions.modes.iter().find(|(rect, _)| contains(rect, column, row)).cloned() {
        app.set_input_mode(mode);
        return;
    }
    if let Some((_, tab)) = app.regions.sidebar_tabs.iter().find(|(rect, _)| contains(rect, column, row)).cloned() {
        app.sidebar_tab = tab;
        return;
    }
    if let Some((_, id)) = app.regions.chats.iter().find(|(rect, _)| contains(rect, column, row)).cloned() {
        app.open_chat(&id);
        return;
    }
    if let Some((_, name)) = app.regions.names.iter().find(|(rect, _)| contains(rect, column, row)).cloned() {
        app.open_private(&name);
        return;
    }
    if let Some((_, link, key)) = app.regions.links.iter().find(|(rect, _, _)| contains(rect, column, row)).cloned() {
        app.open_link(&link, key);
        return;
    }
    if contains(&app.regions.title, column, row) {
        app.title_expanded = !app.title_expanded;
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::config::Config;

    fn app() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx);
        app.regions.chat = Rect { x: 10, y: 2, width: 40, height: 3 };
        app.regions.chat_text = vec!["hello world".into(), "second line  ".into(), String::new()];
        app
    }

    #[test]
    fn selection_text_follows_columns_and_rows() {
        let mut app = app();
        app.selection = Some(Selection { start: (16, 2), end: (20, 2) });
        assert_eq!(app.selected_text(), "world");
        app.selection = Some(Selection { start: (16, 2), end: (15, 3) });
        assert_eq!(app.selected_text(), "world\nsecond");
        app.selection = Some(Selection { start: (15, 3), end: (16, 2) });
        assert_eq!(app.selected_text(), "world\nsecond");
    }

    #[test]
    fn ctrl_v_pastes_the_internal_clipboard() {
        let mut app = app();
        app.copy_to_clipboard("copied");
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        on_key(&mut app, key);
        assert_eq!(app.input.text(), "copied");
    }

    #[test]
    fn tab_with_empty_input_cycles_the_focus() {
        let mut app = app();
        app.regions.sidebar_tabs.push((Rect::default(), SidebarTab::Friends));
        for expected in [Focus::Chat, Focus::Sidebar, Focus::Input] {
            on_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            assert_eq!(app.focus, expected);
        }
        app.focus = Focus::Chat;
        on_key(&mut app, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!((app.focus, app.input.text()), (Focus::Input, "x"));
    }
}
