//! Drawing of the whole interface.

use chrono::{DateTime, Local};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus, InputMode, Severity, SidebarTab, TITLE};
use crate::bbcode::{split_lines, to_segs, Seg};
use crate::images::{self, full_image_url, Key, Kind, Status};
use crate::commands::{command_usage, matching_commands, resolve_alias, COMMAND_INFO, FOOTER_KEYS};
use crate::pager::RowKind;
use crate::rows::{char_width, text_width, wrap};
use crate::sanitize::html_unescape;
use crate::screens::{ColorEditor, ColorPart, Hit, ProfileScreen, ProfileTab, Screen, SettingsEdit, SettingsScreen, Tab, PROFILE_TABS};
use crate::store::{CHANNEL, CONSOLE, USER};
use crate::style::{bold, color, dim, reversed, status_style, status_symbol};

const LIST_WIDTH: u16 = 26;
const SIDEBAR_WIDTH: u16 = 26;
const MAX_INPUT_ROWS: usize = 8;

pub(crate) fn spans(segs: &[Seg]) -> Line<'static> {
    Line::from(segs.iter().map(|seg| Span::styled(seg.text.clone(), seg.style)).collect::<Vec<_>>())
}

pub(crate) fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect { x, y, width, height }
}

pub(crate) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    rect(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height)
}

/// Makes the text drawn in `area` selectable with the mouse, read back from the screen.
pub(crate) fn selectable(frame: &mut Frame, app: &mut App, area: Rect) {
    let buffer = frame.buffer_mut();
    app.regions.modal_text = (area.y..area.y + area.height)
        .map(|y| (area.x..area.x + area.width).filter_map(|x| buffer.cell((x, y)).map(|cell| cell.symbol().to_string())).collect::<String>().trim_end().to_string())
        .collect();
    app.regions.modal_text_area = area;
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.regions.links.clear();
    app.regions.modal_links.clear();
    app.regions.modal_text_area = Rect::default();
    app.regions.modal_text.clear();
    app.regions.chats.clear();
    app.regions.names.clear();
    app.regions.modes.clear();
    app.regions.sidebar_tabs.clear();
    app.regions.footer.clear();
    app.images.begin_frame();

    let (input_rows, cursor, input_lines) = input_layout(app, area.width.saturating_sub(2) as usize);
    let input_height = input_rows as u16 + 2;
    let title_height = if app.title_expanded { 2 } else { 1 };
    let hints = hint_lines(app);
    let hint_height = (hints.len() as u16).min(area.height.saturating_sub(title_height + input_height + 4));
    let [title_area, middle, hint_area, mode_area, input_area, footer_area] = Layout::vertical([
        Constraint::Length(title_height),
        Constraint::Min(3),
        Constraint::Length(hint_height),
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_title(frame, app, title_area);

    let list_width = if area.width >= 60 { LIST_WIDTH } else { 0 };
    let sidebar_width = if app.show_sidebar && area.width >= 90 { SIDEBAR_WIDTH } else { 0 };
    let [list_area, chat_area, sidebar_area] =
        Layout::horizontal([Constraint::Length(list_width), Constraint::Min(10), Constraint::Length(sidebar_width)]).areas(middle);

    if list_width > 0 {
        draw_chat_list(frame, app, list_area);
    }
    draw_chat(frame, app, chat_area);
    if sidebar_width > 0 {
        draw_sidebar(frame, app, sidebar_area);
    }
    if hint_height > 0 {
        frame.render_widget(Paragraph::new(hints).style(dim()), hint_area);
    }
    draw_mode_row(frame, app, mode_area);
    let input_cursor = draw_input(frame, app, input_area, input_lines, cursor);
    draw_footer(frame, app, footer_area);
    draw_toasts(frame, app, middle);

    if let Some(index) = app.screens.len().checked_sub(1) {
        draw_screen(frame, app, index, area);
    } else if let (Some(position), Focus::Input) = (input_cursor, app.focus) {
        frame.set_cursor_position(position);
    }
    highlight_selection(frame, app);
    app.images.end_frame();
    app.images.draw_erased(frame);
    crate::colors::apply_background(frame.buffer_mut());
}

// ------------------------------------------------------------------ title

fn draw_title(frame: &mut Frame, app: &mut App, area: Rect) {
    app.regions.title = area;
    let online = if app.online { color("online") } else { color("offline") };
    let line = Line::from(vec![
        Span::styled(format!(" {TITLE} "), bold()),
        Span::styled("\u{2014} ", dim()),
        Span::styled(app.sub_title.clone(), online),
    ]);
    let mut lines = vec![line];
    if app.title_expanded {
        let message = app.own_status.message.clone();
        lines.push(if message.is_empty() {
            Line::from(Span::styled(" (no status message)", dim()))
        } else {
            let no_names = |_: &str| Style::default();
            Line::from(
                std::iter::once(Span::raw(" "))
                    .chain(to_segs(&message, &no_names, true).into_iter().map(|seg| Span::styled(seg.text.replace('\n', " "), seg.style)))
                    .collect::<Vec<_>>(),
            )
        });
    }
    frame.render_widget(Paragraph::new(lines).style(reversed()), area);
}

// ------------------------------------------------------------------ chat list

fn draw_chat_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let ids = app.list_ids();
    let height = area.height as usize;
    let active = app.active_chat.as_ref().and_then(|id| ids.iter().position(|other| other.as_ref() == Some(id))).unwrap_or(0);
    let start = if ids.len() > height { (active + 1).saturating_sub(height) } else { 0 };
    let inner = rect(area.x, area.y, area.width.saturating_sub(1), area.height);
    let mut lines = Vec::new();
    for (row, id) in ids.iter().enumerate().skip(start).take(height) {
        let Some(id) = id else {
            lines.push(Line::default());
            continue;
        };
        let Some(chat) = app.store.get(id) else { continue };
        let prefix = match chat.kind {
            CHANNEL => "#",
            USER => "@",
            _ => "",
        };
        let pinned = if app.pins.is_pinned(chat.kind, &chat.key) { "\u{2022}" } else { " " };
        let mut label = format!("{pinned}{prefix}{}", html_unescape(&chat.title));
        let max = inner.width as usize;
        let selected = app.active_chat.as_deref() == Some(id.as_str());
        // private chats: a filled circle in the status color when online, an empty one when not
        let presence = (chat.kind == USER).then(|| match app.presence.status_of(&chat.key) {
            Some((status, _)) => (" \u{25cf}", status_style(&status)),
            None => (" \u{25cb}", dim()),
        });
        let unread = if chat.unread { " \u{25cf}" } else { "" };
        let reserved = presence.map_or(0, |(mark, _)| text_width(mark)) + text_width(unread);
        while text_width(&label) > max.saturating_sub(2 + reserved) {
            label.pop();
        }
        let mut style = if chat.kind == USER { app.name_style(&chat.key) } else { Style::default() };
        if chat.unread {
            style = style.patch(bold());
        }
        let selection = if selected { reversed() } else { Style::default() };
        let mut spans = vec![Span::styled(label.clone(), style.patch(selection))];
        if let Some((mark, mark_style)) = presence {
            // In the selected row the circle keeps its color on the row's background (the name
            // color, since the row is reversed) instead of being reversed itself.
            let circle = mark.trim_start();
            let mark_style = if selected { mark_style.bg(style.fg.unwrap_or(ratatui::style::Color::Gray)) } else { mark_style };
            spans.push(Span::styled(" ", style.patch(selection)));
            spans.push(Span::styled(circle, mark_style));
        }
        if !unread.is_empty() {
            spans.push(Span::styled(unread, style.patch(selection)));
        }
        if selected {
            let pad = max.saturating_sub(text_width(&label) + reserved);
            spans.push(Span::styled(" ".repeat(pad), style.patch(selection)));
        }
        app.regions.chats.push((rect(inner.x, inner.y + (row - start) as u16, inner.width, 1), id.clone()));
        lines.push(Line::from(spans));
    }
    let border = if app.focus == Focus::List { color("focus") } else { dim() };
    let block = Block::default().borders(Borders::RIGHT).border_style(border);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

// ------------------------------------------------------------------ chat

fn draw_chat(frame: &mut Frame, app: &mut App, area: Rect) {
    app.regions.chat = area;
    app.regions.chat_text = vec![String::new(); area.height as usize];
    let width = area.width.saturating_sub(1) as usize;
    let height = area.height as usize;
    app.pager.image_rows = if app.images.enabled() { app.images.eicon_rows() as usize } else { 0 };
    let Some(rows) = app.pager_op(|pager, src| pager.rows(src, width, height)) else {
        let text = "No chat open. Use /join <channel> or ctrl+o for the channel list.";
        frame.render_widget(Paragraph::new(text).style(dim()), area);
        return;
    };
    let scroll = app.pager.scroll;
    let end = rows.len().saturating_sub(scroll);
    let start = end.saturating_sub(height);
    let offset = height.saturating_sub(end - start) as u16;
    let mut lines = Vec::new();
    let image_rows = app.pager.image_rows;
    let mut strips: Vec<ChatStrip> = Vec::new();
    for (index, row) in rows[start..end].iter().enumerate() {
        let y = area.y + offset + index as u16;
        if row.kind == RowKind::Image {
            strips.push((y, row.images.clone(), index + image_rows <= end - start, row.slots.clone()));
        }
        let mut x = 0usize;
        for seg in &row.segs {
            let seg_width = text_width(&seg.text);
            if let Some(link) = &seg.link {
                app.regions.links.push((rect(area.x + x as u16, y, seg_width as u16, 1), link.clone(), row.key.clone()));
            }
            x += seg_width;
        }
        app.regions.chat_text[offset as usize + index] = row.segs.iter().map(|seg| seg.text.as_str()).collect();
        lines.push(if row.kind == RowKind::Blank { Line::default() } else { spans(&row.segs) });
    }
    let mut padded: Vec<Line> = vec![Line::default(); offset as usize];
    padded.extend(lines);
    frame.render_widget(Paragraph::new(padded), rect(area.x, area.y, area.width.saturating_sub(1), area.height));
    let draw_images = app.screens.is_empty();
    for (y, keys, complete, slots) in strips {
        let items: Vec<(Key, Option<String>)> = keys
            .into_iter()
            .map(|key| {
                let link = if key.kind == Kind::Avatar { crate::bbcode::profile_url(&key.name) } else { crate::bbcode::eicon_url(&key.name) };
                (key, Some(link))
            })
            .collect();
        let target = StripTarget { x: area.x, y, width: area.width.saturating_sub(1), rows: image_rows as u16, draw: complete && draw_images, chat: true, gap: 0, slots };
        draw_strip(frame, app, &items, &target);
    }
    if scroll > 0 {
        let text = " \u{2193} scrolled (page down) ";
        let w = text_width(text) as u16;
        let marker = rect(area.x + area.width.saturating_sub(w + 1), area.y + area.height.saturating_sub(1), w.min(area.width), 1);
        frame.render_widget(Paragraph::new(text).style(reversed()), marker);
    }
}

type ChatStrip = (u16, Vec<Key>, bool, Vec<(u16, u16)>);

struct StripTarget {
    x: u16,
    y: u16,
    width: u16,
    rows: u16,
    draw: bool,
    chat: bool,
    /// Empty columns between images (0 in the chat so that split pictures join).
    gap: u16,
    /// Fixed (column, width) of each image inside a text line; empty = side by side.
    slots: Vec<(u16, u16)>,
}

/// Draws images side by side in a strip of `rows` rows; not yet loaded or missing images show their name.
fn draw_strip(frame: &mut Frame, app: &mut App, items: &[(Key, Option<String>)], target: &StripTarget) {
    let mut x = target.x;
    let limit = target.x + target.width;
    for (index, (key, link)) in items.iter().enumerate() {
        app.images.request(key);
        let slot = target.slots.get(index).copied();
        if let Some((column, _)) = slot {
            x = target.x + column;
        }
        let (columns, rows) = match (app.images.status(key), app.images.size_cells(key)) {
            (Status::Ready, Some(size)) => size,
            _ => (slot.map_or_else(|| text_width(&key.label()).min(target.width as usize) as u16, |(_, width)| width), 1),
        };
        if columns == 0 || (slot.is_none() && x + columns > limit) || x + columns > target.x + target.width {
            break;
        }
        let area = rect(x, target.y, columns, rows.min(target.rows));
        if target.draw {
            let drawn = app.images.status(key) == Status::Ready && app.images.render(frame, key, area);
            if !drawn {
                let style = match (app.images.status(key), key.kind) {
                    (Status::Loading, _) => dim(),
                    (_, Kind::Eicon) => crate::style::eicon_style(),
                    _ => crate::style::icon_style(),
                };
                frame.render_widget(Paragraph::new(Span::styled(key.label(), style)), rect(x, target.y, columns, 1));
            }
        }
        if let Some(link) = link {
            if target.chat {
                app.regions.links.push((area, link.clone(), None));
            } else {
                app.regions.modal_links.push((area, link.clone()));
            }
        }
        x += columns + target.gap;
    }
}

// ------------------------------------------------------------------ sidebar

fn draw_sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    let border = if app.focus == Focus::Sidebar { color("focus") } else { dim() };
    let block = Block::default().borders(Borders::LEFT).border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let tabs = [(SidebarTab::Friends, "Friends"), (SidebarTab::Channel, "Channel")];
    let mut x = inner.x;
    let mut spans_row = Vec::new();
    for (tab, label) in tabs {
        let text = format!(" {label} ");
        let w = text.len() as u16;
        let style = if app.sidebar_tab == tab { reversed() } else { dim() };
        app.regions.sidebar_tabs.push((rect(x, inner.y, w, 1), tab));
        spans_row.push(Span::styled(text, style));
        x += w;
    }
    frame.render_widget(Paragraph::new(Line::from(spans_row)), rect(inner.x, inner.y, inner.width, 1));

    let body = rect(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::new();
    let push_entry = |app: &mut App, lines: &mut Vec<Line>, name: &str, status: &str, is_op: bool| {
        if lines.len() >= body.height as usize {
            return;
        }
        let y = body.y + lines.len() as u16;
        let selected = app.focus == Focus::Sidebar && app.regions.names.len() == app.sidebar_cursor;
        app.regions.names.push((rect(body.x, y, body.width, 1), name.to_string()));
        let mut label = name.to_string();
        let max = (body.width as usize).saturating_sub(3);
        while text_width(&label) > max {
            label.pop();
        }
        let marker = if is_op { "\u{2605}" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", status_symbol(status)), status_style(status)),
            Span::styled(label, if selected { app.name_style(name).patch(reversed()) } else { app.name_style(name) }),
            Span::styled(marker.to_string(), color("op")),
        ]));
    };
    match app.sidebar_tab {
        SidebarTab::Friends => {
            let friends = app.presence.online_friends();
            let bookmarks = app.presence.online_bookmarks();
            let header = |text: &str| Line::from(Span::styled(text.to_string(), dim()));
            if !friends.is_empty() {
                lines.push(header("Friends"));
                for entry in &friends {
                    push_entry(app, &mut lines, &entry.name, &entry.status, false);
                }
            }
            if !bookmarks.is_empty() {
                lines.push(header("Bookmarks"));
                for entry in &bookmarks {
                    push_entry(app, &mut lines, &entry.name, &entry.status, false);
                }
            }
            if lines.is_empty() {
                lines.push(Line::from(Span::styled("nobody online", dim())));
            }
        }
        SidebarTab::Channel => {
            let members = app.current_chat().filter(|chat| chat.kind == CHANNEL).and_then(|chat| app.presence.channel_members(&chat.key));
            match members {
                Some(members) => {
                    lines.push(Line::from(Span::styled(format!("{} users", members.len()), dim())));
                    for entry in &members {
                        push_entry(app, &mut lines, &entry.name, &entry.status, entry.is_op);
                    }
                }
                None => lines.push(Line::from(Span::styled("no channel open", dim()))),
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), body);
}

// ------------------------------------------------------------------ input

fn input_layout(app: &App, width: usize) -> (usize, (usize, usize), Vec<String>) {
    let width = width.max(1);
    let text = app.input.text();
    let cursor_index = app.input.cursor();
    let mut lines = Vec::new();
    let mut line = String::new();
    let (mut row, mut col) = (0usize, 0usize);
    let mut cursor = (0usize, 0usize);
    let total = text.chars().count();
    for (index, c) in text.chars().enumerate() {
        if c == '\n' {
            if index == cursor_index {
                cursor = (row, col);
            }
            lines.push(std::mem::take(&mut line));
            row += 1;
            col = 0;
            continue;
        }
        let w = char_width(c);
        if col + w > width {
            lines.push(std::mem::take(&mut line));
            row += 1;
            col = 0;
        }
        if index == cursor_index {
            cursor = (row, col);
        }
        line.push(c);
        col += w;
    }
    if cursor_index == total {
        if col >= width {
            lines.push(std::mem::take(&mut line));
            row += 1;
            col = 0;
        }
        cursor = (row, col);
    }
    lines.push(line);
    let rows = lines.len().clamp(1, MAX_INPUT_ROWS);
    (rows, cursor, lines)
}

fn draw_input(frame: &mut Frame, app: &mut App, area: Rect, lines: Vec<String>, cursor: (usize, usize)) -> Option<(u16, u16)> {
    let title = match (app.input_mode, app.current_chat()) {
        (InputMode::Ads, _) => " Ad ".to_string(),
        (_, Some(chat)) if chat.kind == CONSOLE => " Console (commands only) ".to_string(),
        (_, Some(chat)) => format!(" {} ", html_unescape(&chat.title)),
        (_, None) => String::new(),
    };
    let border = if app.focus != Focus::Input {
        dim()
    } else if app.input_mode == InputMode::Ads {
        color("ad")
    } else {
        Style::default()
    };
    let block = Block::default().borders(Borders::ALL).border_style(border).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    crate::colors::fill(frame, inner, "surface");

    let visible = inner.height as usize;
    let first = (cursor.0 + 1).saturating_sub(visible);
    let shown: Vec<Line> = if app.input.text().is_empty() {
        let hint = if app.input_mode == InputMode::Ads { "Write an ad, enter posts it" } else { "Message, /help for commands" };
        vec![Line::from(Span::styled(hint, dim()))]
    } else {
        lines.iter().skip(first).take(visible).map(|line| Line::from(line.clone())).collect()
    };
    frame.render_widget(Paragraph::new(shown), inner);
    let x = inner.x + cursor.1 as u16;
    let y = inner.y + (cursor.0 - first) as u16;
    (x < inner.x + inner.width && y < inner.y + inner.height).then_some((x, y))
}

fn draw_mode_row(frame: &mut Frame, app: &mut App, area: Rect) {
    let mut x = area.x;
    let mut parts = Vec::new();
    for (mode, label) in [(InputMode::Chat, " Chat "), (InputMode::Ads, " Ads ")] {
        let style = if app.input_mode == mode { reversed() } else { dim() };
        app.regions.modes.push((rect(x, area.y, label.len() as u16, 1), mode));
        parts.push(Span::styled(label, style));
        x += label.len() as u16;
    }
    let mut right = String::new();
    if let Some(limit) = app.input_limit() {
        let length = app.input.char_count();
        if length * 10 >= limit * 8 || app.input_mode == InputMode::Ads {
            right = format!("{length}/{limit}");
        }
    }
    if app.focus != Focus::Input {
        let name = match app.focus {
            Focus::List => "chat list",
            Focus::Chat => "chat (scroll with arrows)",
            _ => "users (enter: open chat, left/right: tab)",
        };
        right = format!("focus: {name} \u{2014} tab: next, esc: input");
    } else if app.input_mode == InputMode::Ads && !app.input.text().starts_with('/') {
        if let Some(key) = app.ad_channel.clone() {
            let wait = app.ad_wait_seconds(&key);
            if wait > 0 {
                right = format!("next ad in {}:{:02}   {right}", wait / 60, wait % 60);
            }
        }
    }
    let used = x - area.x;
    let room = area.width.saturating_sub(used + 1) as usize;
    while text_width(&right) > room {
        right.pop();
    }
    let padding = room.saturating_sub(text_width(&right));
    parts.push(Span::raw(" ".repeat(padding + 1)));
    parts.push(Span::styled(right, dim()));
    frame.render_widget(Paragraph::new(Line::from(parts)), area);
}

fn highlight_selection(frame: &mut Frame, app: &App) {
    let Some(selection) = &app.selection else { return };
    let ((c0, r0), (c1, r1)) = selection.ordered();
    let area = app.selection_area().0;
    let last_column = area.x + area.width.saturating_sub(1);
    let buffer = frame.buffer_mut();
    for y in r0..=r1 {
        let from = if y == r0 { c0 } else { area.x };
        let to = if y == r1 { c1 } else { last_column };
        for x in from..=to.min(last_column) {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_style(reversed());
            }
        }
    }
}

// ------------------------------------------------------------------ footer

fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) {
    let bar = color("footer");
    let key_style = bar.patch(color("footer_key"));
    let mut x = area.x;
    let mut parts = Vec::new();
    for (key, label) in FOOTER_KEYS {
        let key_text = format!(" ^{key} ");
        let label_text = format!("{label} ");
        let width = (text_width(&key_text) + text_width(&label_text)) as u16;
        if x + width > area.x + area.width {
            break;
        }
        app.regions.footer.push((rect(x, area.y, width, 1), *key));
        parts.push(Span::styled(key_text, key_style));
        parts.push(Span::styled(label_text, bar));
        x += width;
    }
    frame.render_widget(Paragraph::new(Line::from(parts)).style(bar), area);
}

// ------------------------------------------------------------------ command hints

const MAX_HINTS: usize = 12;

/// The hint panel above the input: matching commands while typing the command word,
/// the usage of the command once a space was typed.
fn hint_lines(app: &App) -> Vec<Line<'static>> {
    let value = app.input.text();
    let Some(rest) = value.strip_prefix('/') else { return Vec::new() };
    if rest.contains(char::is_whitespace) {
        let word = rest.split(char::is_whitespace).next().unwrap_or("");
        let name = resolve_alias(word);
        return match COMMAND_INFO.iter().find(|entry| entry.0 == name) {
            Some((name, args, description)) => {
                let args = if args.is_empty() { String::new() } else { format!(" {args}") };
                vec![Line::from(vec![Span::styled(format!(" /{name}{args}"), bold()), Span::styled(format!("  {description}"), dim())])]
            }
            None => Vec::new(),
        };
    }
    let typed = rest.to_lowercase();
    let matches = matching_commands(&typed);
    if matches.is_empty() {
        return Vec::new();
    }
    let hidden = if matches.len() > MAX_HINTS { matches.len() - MAX_HINTS + 1 } else { 0 };
    let shown = &matches[..matches.len() - hidden];
    let width = shown.iter().map(|(name, args, _)| command_usage(name, args).chars().count()).max().unwrap_or(0) + 2;
    let mut lines: Vec<Line> = shown
        .iter()
        .map(|(name, args, description)| {
            let usage = command_usage(name, args);
            let split = typed.chars().count().min(name.chars().count());
            let (head, tail): (String, String) = (name.chars().take(split).collect(), name.chars().skip(split).collect());
            let args = usage.chars().skip(name.chars().count() + 1).collect::<String>();
            let padding = width.saturating_sub(usage.chars().count());
            Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("/{head}"), bold()),
                Span::raw(format!("{tail}{args}{}", " ".repeat(padding))),
                Span::styled(description.to_string(), dim()),
            ])
        })
        .collect();
    if hidden > 0 {
        lines.push(Line::from(Span::styled(format!(" ... {hidden} more, keep typing"), dim())));
    }
    lines
}

// ------------------------------------------------------------------ toasts

fn draw_toasts(frame: &mut Frame, app: &App, area: Rect) {
    let width = 50u16.min(area.width.saturating_sub(2));
    if width < 12 {
        return;
    }
    let mut y = area.y;
    for toast in app.toasts.iter().rev() {
        let inner_width = (width - 2) as usize;
        let lines = wrap(&[Seg::new(toast.text.clone(), Style::default())], inner_width);
        let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(y - area.y));
        if height < 3 {
            break;
        }
        let border = match toast.severity {
            Severity::Information => color("information"),
            Severity::Warning => color("warning"),
            Severity::Error => color("error"),
        };
        let toast_area = rect(area.x + area.width - width - 1, y, width, height);
        let mut block = Block::default().borders(Borders::ALL).border_style(border);
        if !toast.title.is_empty() {
            block = block.title(Span::styled(format!(" {} ", toast.title), bold()));
        }
        frame.render_widget(Clear, toast_area);
        crate::colors::fill(frame, toast_area, "panel");
        frame.render_widget(Paragraph::new(lines.iter().map(|row| spans(row)).collect::<Vec<_>>()).block(block), toast_area);
        y += height;
    }
}

// ------------------------------------------------------------------ screens

pub(crate) fn draw_lines(frame: &mut Frame, app: &mut App, area: Rect, lines: &[Vec<Seg>], scroll: &mut usize) {
    let width = area.width as usize;
    let wrapped: Vec<Vec<Seg>> = lines.iter().flat_map(|line| wrap(line, width)).collect();
    *scroll = (*scroll).min(wrapped.len().saturating_sub(area.height as usize));
    let mut shown = Vec::new();
    for (index, row) in wrapped.iter().skip(*scroll).take(area.height as usize).enumerate() {
        let y = area.y + index as u16;
        let mut x = 0usize;
        for seg in row {
            let w = text_width(&seg.text);
            if let Some(link) = &seg.link {
                app.regions.modal_links.push((rect(area.x + x as u16, y, w as u16, 1), link.clone()));
            }
            x += w;
        }
        shown.push(spans(row));
    }
    frame.render_widget(Paragraph::new(shown), area);
}

pub(crate) fn modal(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    crate::colors::fill(frame, area, "panel");
    let block = Block::default().borders(Borders::ALL).border_style(color("focus")).title(Span::styled(format!(" {title} "), bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

pub(crate) fn footer(frame: &mut Frame, area: Rect, text: &str) {
    let line = rect(area.x + 2, area.y + area.height - 1, area.width.saturating_sub(4), 1);
    frame.render_widget(Paragraph::new(Span::styled(format!(" {text} "), dim())), line);
}

/// Breaks the text of an input into rows of at most `columns` cells; also returns the row and
/// column of the cursor.
pub(crate) fn wrap_input(text: &str, cursor_index: usize, columns: usize) -> (Vec<String>, (usize, usize)) {
    let mut lines: Vec<String> = vec![String::new()];
    let mut used = 0usize;
    let mut cursor_at = (0usize, 0usize);
    let mut count = 0usize;
    for (index, c) in text.chars().enumerate() {
        count += 1;
        if c == '\n' {
            if index == cursor_index {
                cursor_at = (lines.len() - 1, used);
            }
            lines.push(String::new());
            used = 0;
            continue;
        }
        let w = char_width(c);
        if used + w > columns {
            lines.push(String::new());
            used = 0;
        }
        if index == cursor_index {
            cursor_at = (lines.len() - 1, used);
        }
        lines.last_mut().unwrap().push(c);
        used += w;
    }
    if cursor_index >= count {
        if used >= columns {
            lines.push(String::new());
            used = 0;
        }
        cursor_at = (lines.len() - 1, used);
    }
    (lines, cursor_at)
}

fn draw_screen(frame: &mut Frame, app: &mut App, index: usize, area: Rect) {
    let mut screen = app.screens.remove(index);
    let mut cursor = None;
    match &mut screen {
        Screen::Login(login) => {
            let outer = centered(area, 54, 12);
            let inner = modal(frame, outer, "Login");
            let field = |label: &str, value: String, focused: bool| {
                let style = if focused { reversed() } else { Style::default() };
                Line::from(vec![Span::styled(format!("{label:<10}"), dim()), Span::styled(format!(" {value:<30} "), style)])
            };
            let remember = format!("[{}] remember password", if login.remember { "x" } else { " " });
            let lines = vec![
                Line::default(),
                field("Account", login.account.clone(), login.focus == 0),
                field("Password", "*".repeat(login.password.chars().count()), login.focus == 1),
                Line::from(vec![Span::raw("           "), Span::styled(remember, if login.focus == 2 { reversed() } else { Style::default() })]),
                Line::default(),
                Line::from(Span::styled(login.message.clone(), color("warning"))),
            ];
            frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), inner);
            footer(frame, outer, "tab: next field, enter: log in, space: toggle");
            if login.focus < 2 && !login.busy {
                let len = if login.focus == 0 { login.account.chars().count() } else { login.password.chars().count() };
                cursor = Some((inner.x + 12 + len.min(30) as u16, inner.y + 1 + login.focus as u16));
            }
        }
        Screen::Character(chars) => {
            let height = (chars.names.len() as u16 + 4).min(area.height.saturating_sub(2)).max(6);
            let outer = centered(area, 44, height);
            let inner = modal(frame, outer, "Choose a character");
            let visible = inner.height.saturating_sub(1) as usize;
            let start = (chars.selected + 1).saturating_sub(visible);
            let lines: Vec<Line> = chars
                .names
                .iter()
                .enumerate()
                .skip(start)
                .take(visible)
                .map(|(i, name)| Line::from(Span::styled(format!(" {name:<38}"), if i == chars.selected { reversed() } else { Style::default() })))
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
            footer(frame, outer, "enter: choose, esc: cancel");
        }
        Screen::Profile(profile) => draw_profile(frame, app, area, profile),
        Screen::Info(info) => {
            let width = if info.wide { (area.width * 8 / 10).max(40) } else { 70 };
            let wrapped: usize = info.lines.iter().map(|line| wrap(line, width.saturating_sub(4) as usize).len()).sum();
            let outer = centered(area, width, (wrapped as u16 + 3).min(area.height.saturating_sub(2)).max(5));
            let inner = modal(frame, outer, &info.title.clone());
            let body = rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height);
            let lines = info.lines.clone();
            draw_lines(frame, app, body, &lines, &mut info.scroll);
            footer(frame, outer, "esc: close");
        }
        Screen::Picker(picker) => {
            let outer = centered(area, 78, area.height.saturating_sub(4).max(8));
            let inner = modal(frame, outer, &picker.title.clone());
            let filter = Line::from(vec![Span::styled("Search: ", dim()), Span::raw(picker.filter.clone())]);
            frame.render_widget(Paragraph::new(filter), rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1));
            let list = rect(inner.x + 1, inner.y + 2, inner.width.saturating_sub(2), inner.height.saturating_sub(3));
            let visible = picker.visible();
            let capacity = list.height as usize;
            let start = (picker.selected + 1).saturating_sub(capacity);
            let lines: Vec<Line> = visible
                .iter()
                .enumerate()
                .skip(start)
                .take(capacity)
                .map(|(i, item)| {
                    let mut label = item.label.clone();
                    while text_width(&label) > list.width as usize {
                        label.pop();
                    }
                    Line::from(Span::styled(label, if i == picker.selected { reversed() } else { Style::default() }))
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), list);
            footer(frame, outer, "type to search, enter: choose, esc: close");
            cursor = Some((inner.x + 9 + text_width(&picker.filter) as u16, inner.y));
        }
        Screen::Edit(edit) => {
            let (lines, cursor_at) = wrap_input(edit.input.text(), edit.input.cursor(), 60);
            let outer = centered(area, 64, (lines.len() as u16 + 4).min(area.height));
            let inner = modal(frame, outer, &edit.title.clone());
            crate::colors::fill(frame, outer, "status_box");
            let field = rect(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), (lines.len() as u16).min(inner.height.saturating_sub(1)));
            crate::colors::fill(frame, field, "status_field");
            for (row, line) in lines.iter().enumerate() {
                frame.render_widget(Paragraph::new(line.clone()), rect(inner.x + 1, inner.y + 1 + row as u16, inner.width.saturating_sub(2), 1));
            }
            let count = format!("{}/{}", edit.input.char_count(), edit.max);
            frame.render_widget(Paragraph::new(Span::styled(count, dim())), rect(inner.x + 1, inner.y + 1 + lines.len() as u16, inner.width.saturating_sub(2), 1));
            cursor = Some((inner.x + 1 + cursor_at.1 as u16, inner.y + 1 + cursor_at.0 as u16));
            footer(frame, outer, "enter: save, ctrl+enter / ctrl+j: new line, ctrl+e: emoji, esc: cancel, ctrl+u: clear");
        }
        Screen::Settings(settings) => cursor = draw_settings(frame, area, settings),
        Screen::Notes(notes) => cursor = crate::notes_view::draw(frame, app, area, notes),
        Screen::Friends(friends) => cursor = crate::friends_view::draw(frame, app, area, friends),
    }
    if let Some((x, y)) = cursor {
        frame.set_cursor_position((x, y));
    }
    app.screens.insert(index, screen);
}

// ------------------------------------------------------------------ settings

const SETTING_LABEL: usize = 22;
const SETTING_VALUE: usize = 26;
const COLOR_LABEL: usize = 14;
const COLOR_VALUE: usize = 30;

/// Cuts text to a width in cells.
fn fit(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out
}

fn pad(text: &str, width: usize) -> String {
    let text = fit(text, width);
    let fill = width.saturating_sub(text_width(&text));
    format!("{text}{}", " ".repeat(fill))
}

/// Draws the settings page, returns where the cursor goes. Fills the click areas of the screen.
fn draw_settings(frame: &mut Frame, area: Rect, settings: &mut SettingsScreen) -> Option<(u16, u16)> {
    let outer = centered(area, area.width.saturating_sub(4).clamp(40, 100), area.height.saturating_sub(2).max(14));
    let inner = modal(frame, outer, "Settings");
    crate::colors::fill(frame, outer, "settings_box");
    let body = rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height.saturating_sub(1));
    let selected_style = color("settings_selected");
    let group_style = color("settings_group");
    settings.hits.clear();

    let mut x = body.x;
    let mut tabs = Vec::new();
    for (label, tab) in [("General", Tab::General), ("Colors", Tab::Colors)] {
        let text = format!(" {label} ");
        let width = text_width(&text) as u16;
        settings.hits.push((rect(x, body.y, width, 1), Hit::Tab(tab)));
        tabs.push(Span::styled(text, if settings.tab == tab { selected_style } else { dim() }));
        tabs.push(Span::raw("  "));
        x += width + 2;
    }
    frame.render_widget(Paragraph::new(Line::from(tabs)), rect(body.x, body.y, body.width, 1));
    let (button, button_style) = if settings.confirm_reset { (" Reset? click again or y ", color("warning")) } else { (" Reset ", dim()) };
    let button = format!("[{button}]");
    let width = text_width(&button) as u16;
    if body.width > x - body.x + width {
        let place = rect(body.x + body.width - width, body.y, width, 1);
        settings.hits.push((place, Hit::Reset));
        frame.render_widget(Paragraph::new(Span::styled(button, button_style)), place);
    }

    let editor_rows: u16 = match &settings.edit {
        Some(SettingsEdit::Color(editor)) => editor.part_list().len() as u16 + 3,
        _ => 0,
    };
    let list = rect(body.x, body.y + 2, body.width, body.height.saturating_sub(3 + editor_rows));
    let mut edit_at: Option<(u16, usize)> = None;
    let mut rows: Vec<(Option<usize>, Line)> = Vec::new();
    let selected = settings.current();
    let mut group = "";
    match settings.tab {
        Tab::General => {
            for (index, setting) in crate::settings::SETTINGS.iter().enumerate() {
                if setting.group != group {
                    group = setting.group;
                    rows.push((None, Line::from(Span::styled(group.to_string(), group_style))));
                }
                let value = settings.values.get(index).cloned().unwrap_or_default();
                let (shown, shown_style) = match setting.field {
                    crate::settings::Field::Bool => (if value == "true" { "[x]" } else { "[ ]" }.to_string(), Style::default()),
                    _ if value.is_empty() => (if setting.key == "data_dir" { "(default)" } else { "(none)" }.to_string(), dim()),
                    _ => (value, Style::default()),
                };
                let marker = if index == selected { "> " } else { "  " };
                let label_style = if index == selected { selected_style } else { Style::default() };
                let mut spans = vec![Span::raw(marker), Span::styled(pad(setting.key, SETTING_LABEL), label_style), Span::raw(" ")];
                match (&settings.edit, index == selected) {
                    (Some(SettingsEdit::Text(input)), true) => {
                        spans.push(Span::styled(pad(input.text(), SETTING_VALUE), color("focus")));
                        let before: String = input.text().chars().take(input.cursor()).collect();
                        edit_at = Some((SETTING_LABEL as u16 + 3 + text_width(&before) as u16, index));
                    }
                    _ => spans.push(Span::styled(pad(&shown, SETTING_VALUE), shown_style)),
                }
                if !setting.effect.note().is_empty() {
                    spans.push(Span::styled(format!(" {}", setting.effect.note()), dim()));
                }
                rows.push((Some(index), Line::from(spans)));
            }
        }
        Tab::Colors => {
            let palette = crate::colors::palette();
            for (index, entry) in crate::colors::entries().iter().enumerate() {
                if entry.group != group {
                    group = entry.group;
                    rows.push((None, Line::from(Span::styled(group.to_string(), group_style))));
                }
                let name = entry.path.rsplit('.').next().unwrap_or(&entry.path).to_string();
                let marker = if index == selected { "> " } else { "  " };
                let label_style = if index == selected { selected_style } else { Style::default() };
                let spec = palette.spec(&entry.path);
                let shown = if spec.is_empty() && entry.kind == crate::colors::Kind::Background { "(terminal)" } else { spec };
                let preview = match entry.kind {
                    crate::colors::Kind::Background => match palette.background(&entry.path) {
                        Some(background) => Span::styled("          ", Style::default().bg(background)),
                        None => Span::styled("transparent", dim()),
                    },
                    crate::colors::Kind::Style => Span::styled(sample(&entry.path, &name), palette.style(&entry.path)),
                };
                let spans = vec![
                    Span::raw(marker),
                    Span::styled(pad(&name, COLOR_LABEL), label_style),
                    Span::raw(" "),
                    Span::raw(pad(shown, COLOR_VALUE)),
                    Span::raw(" "),
                    preview,
                ];
                rows.push((Some(index), Line::from(spans)));
            }
        }
    }
    let position = rows.iter().position(|(index, _)| *index == Some(selected)).unwrap_or(0);
    let capacity = list.height as usize;
    let mut start = (position + 1).saturating_sub(capacity);
    if start > 0 && rows.get(start.saturating_sub(1)).is_some_and(|(index, _)| index.is_none()) && position + 1 < start + capacity {
        start -= 1;
    }
    for (offset, (index, _)) in rows.iter().skip(start).take(capacity).enumerate() {
        if let Some(index) = index {
            settings.hits.push((rect(list.x, list.y + offset as u16, list.width, 1), Hit::Row(*index)));
        }
    }
    let shown: Vec<Line> = rows.iter().skip(start).take(capacity).map(|(_, line)| line.clone()).collect();
    frame.render_widget(Paragraph::new(shown), list);
    let mut cursor = None;
    if let Some((x, index)) = edit_at {
        let row = rows.iter().position(|(entry, _)| *entry == Some(index)).unwrap_or(0);
        if row >= start && row < start + capacity {
            let y = list.y + (row - start) as u16;
            let field = rect(list.x + SETTING_LABEL as u16 + 3, y, (SETTING_VALUE as u16).min(list.width.saturating_sub(SETTING_LABEL as u16 + 3)), 1);
            crate::colors::fill(frame, field, "settings_field");
            cursor = Some((list.x + x, y));
        }
    }

    if let Some(SettingsEdit::Color(editor)) = &settings.edit {
        let top = list.y + list.height + 1;
        let area = rect(body.x, top, body.width, editor_rows);
        crate::colors::fill(frame, area, "settings_field");
        cursor = draw_color_editor(frame, area, editor, &mut settings.hits);
    }
    if let Some((message, error)) = &settings.message {
        let style = if *error { color("error") } else { color("information") };
        frame.render_widget(Paragraph::new(Span::styled(fit(message, body.width as usize), style)), rect(body.x, body.y + body.height - 1, body.width, 1));
    }
    let hint = match &settings.edit {
        None => "click or up/down: choose, tab: General/Colors, enter: change, r: default, R: reset tab, esc: close",
        Some(SettingsEdit::Text(_)) => "enter: save, esc: cancel, ctrl+u: clear",
        Some(SettingsEdit::Color(_)) => "tab/up/down: part, left/right or click: choose, space: toggle, enter: save, esc: cancel",
    };
    footer(frame, outer, hint);
    let version = format!(" terminalchat {} ", env!("CARGO_PKG_VERSION"));
    let width = text_width(&version) as u16;
    // on the top border, right; the bottom one is full of key hints
    if outer.width > width + 16 {
        frame.render_widget(Paragraph::new(Span::styled(version, dim())), rect(outer.x + outer.width - width - 2, outer.y, width, 1));
    }
    cursor
}

/// Sample text for a style: names stand for themselves, the rest shows a word.
fn sample(path: &str, name: &str) -> String {
    if path.contains('.') {
        return name.to_string();
    }
    match path {
        "url" => "https://f-list.net".into(),
        "footer" | "footer_key" => " ctrl+o ".into(),
        "eicon" | "icon" | "user" | "sender" => "Character".into(),
        "channel_link" => "#Frontpage".into(),
        _ => "Sample text".into(),
    }
}

/// The color editor below the list: text, color pickers, attributes and a preview.
fn draw_color_editor(frame: &mut Frame, area: Rect, editor: &ColorEditor, hits: &mut Vec<(Rect, Hit)>) -> Option<(u16, u16)> {
    let label_width = 12u16;
    let label = |text: &str, part: ColorPart| {
        let style = if editor.part == part { color("settings_selected") } else { dim() };
        Span::styled(pad(text, label_width as usize - 1), style)
    };
    let mut lines = vec![Line::from(Span::styled(format!("Edit {}", editor.path), color("settings_group")))];
    let mut cursor = None;
    for (row, part) in editor.part_list().iter().enumerate() {
        let y = area.y + 1 + row as u16;
        hits.push((rect(area.x, y, label_width, 1), Hit::Part(*part)));
        let mut spans = Vec::new();
        match part {
            ColorPart::Text => {
                hits.push((rect(area.x, y, area.width, 1), Hit::Part(*part)));
                spans.push(label("Text", *part));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(editor.input.text().to_string(), color("focus")));
                if editor.part == ColorPart::Text {
                    let before: String = editor.input.text().chars().take(editor.input.cursor()).collect();
                    cursor = Some((area.x + label_width + text_width(&before) as u16, y));
                }
            }
            ColorPart::Color | ColorPart::Background => {
                let (name, chosen) = if *part == ColorPart::Color {
                    (if editor.kind == crate::colors::Kind::Background { "Background" } else { "Color" }, editor.color)
                } else {
                    ("Background", editor.background)
                };
                spans.push(label(name, *part));
                spans.push(Span::raw(" "));
                let mut x = area.x + label_width;
                let none = if chosen == 0 { "<none>" } else { " none " };
                spans.push(Span::styled(none, if chosen == 0 { color("settings_selected") } else { dim() }));
                hits.push((rect(x, y, 6, 1), Hit::Swatch(*part, 0)));
                x += 6;
                for (index, preset) in crate::colors::PRESETS.iter().enumerate() {
                    let background = crate::colors::parse_hex(preset).unwrap_or_default();
                    let mark = if index + 1 == chosen { "<>" } else { "  " };
                    let ink = if is_light(preset) { ratatui::style::Color::Black } else { ratatui::style::Color::White };
                    spans.push(Span::styled(mark, Style::default().bg(background).fg(ink)));
                    hits.push((rect(x, y, 2, 1), Hit::Swatch(*part, index + 1)));
                    x += 2;
                }
            }
            ColorPart::Modifiers => {
                spans.push(label("Style", *part));
                spans.push(Span::raw(" "));
                let mut x = area.x + label_width;
                for (index, modifier) in crate::colors::MODIFIERS.iter().enumerate() {
                    let check = if editor.parts.has(modifier) { "[x]" } else { "[ ]" };
                    let style = if editor.part == ColorPart::Modifiers && index == editor.modifier { color("settings_selected") } else { Style::default() };
                    let text = format!("{check} {modifier}");
                    let width = text_width(&text) as u16;
                    hits.push((rect(x, y, width, 1), Hit::Modifier(index)));
                    spans.push(Span::styled(text, style));
                    spans.push(Span::raw("  "));
                    x += width + 2;
                }
            }
        }
        lines.push(Line::from(spans));
    }
    let preview = match editor.kind {
        crate::colors::Kind::Background => match crate::colors::parse_hex(editor.input.text().trim()) {
            Some(background) => Span::styled("                    ", Style::default().bg(background)),
            None => Span::styled(if editor.input.text().trim().is_empty() { "transparent" } else { "?" }, dim()),
        },
        crate::colors::Kind::Style => match crate::colors::parse_style(editor.input.text()) {
            Some(style) => Span::styled(sample(&editor.path, editor.path.rsplit('.').next().unwrap_or("")), style),
            None => Span::styled("not readable yet", color("error")),
        },
    };
    lines.push(Line::from(vec![Span::styled(pad("Preview", label_width as usize - 1), dim()), Span::raw(" "), preview]));
    frame.render_widget(Paragraph::new(lines), area);
    cursor
}

/// Light colors get dark marks in the picker.
fn is_light(hex: &str) -> bool {
    match crate::colors::parse_hex(hex) {
        Some(ratatui::style::Color::Rgb(red, green, blue)) => (red as u32 * 299 + green as u32 * 587 + blue as u32 * 114) / 1000 > 140,
        _ => false,
    }
}

// ------------------------------------------------------------------ profile text

fn format_time(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|time| time.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into())
}

/// One row of a profile tab: text, the first row of an image strip, or a row the strip covers.
enum PRow {
    Text(Vec<Seg>),
    /// Images, the strip height in rows and their places (column, width); empty: side by side.
    Strip(Vec<(Key, Option<String>)>, u16, Vec<(u16, u16)>),
    Cont,
}

const PROFILE_GUTTER: usize = 2;
const IMAGE_TEXT_ROWS: usize = 2;
const KINK_TITLES: [(&str, &str); 4] = [("fave", "Favorites"), ("yes", "Yes"), ("maybe", "Maybe"), ("no", "No")];

fn push_strip(rows: &mut Vec<PRow>, items: Vec<(Key, Option<String>)>, height: u16, slots: Vec<(u16, u16)>) {
    rows.push(PRow::Strip(items, height, slots));
    for _ in 1..height {
        rows.push(PRow::Cont);
    }
}

fn heading() -> Style {
    bold().patch(crate::style::underlined())
}

/// Text lines right of the avatar: name and title, status, dates, link.
fn profile_header(app: &App, screen: &ProfileScreen) -> Vec<Vec<Seg>> {
    let name = match &screen.result {
        Some(Ok(profile)) if !profile.name.is_empty() => profile.name.clone(),
        _ => screen.character.clone(),
    };
    let mut first = vec![Seg::new(name.clone(), bold().patch(app.name_style(&name)))];
    let profile = match &screen.result {
        None => return vec![first, vec![Seg::new("Loading ...", dim())]],
        Some(Err(error)) => return vec![first, vec![Seg::new(format!("Could not load the profile: {error}"), color("error"))]],
        Some(Ok(profile)) => profile,
    };
    if !profile.title.is_empty() {
        first.push(Seg::new(format!("  {}", profile.title), crate::style::italic()));
    }
    let mut lines = vec![first];
    let name_style = |name: &str| app.name_style(name);
    match app.presence.status_of(&name) {
        None => lines.push(vec![Seg::new("offline", dim())]),
        Some((status, message)) => {
            let mut segs = vec![Seg::new(status.clone(), status_style(&status))];
            if !message.is_empty() {
                segs.push(Seg::new(": ", Style::default()));
                segs.extend(to_segs(&message, &name_style, true));
            }
            lines.extend(split_lines(&segs));
        }
    }
    lines.push(vec![Seg::new(
        format!("Created {} \u{b7} updated {} \u{b7} {} views", format_time(profile.created), format_time(profile.updated), profile.views),
        dim(),
    )]);
    let url = crate::bbcode::profile_url(&name);
    lines.push(vec![Seg::linked(url.clone(), crate::style::url_style(), url)]);
    lines
}

fn info_rows(app: &App, profile: &fchat::Profile, width: usize) -> Vec<PRow> {
    let name_style = |name: &str| app.name_style(name);
    let mut lines: Vec<Vec<Seg>> = Vec::new();
    for (group, entries) in &profile.infotags {
        if !lines.is_empty() {
            lines.push(Vec::new());
        }
        lines.push(vec![Seg::new(group.clone(), heading())]);
        let label_width = entries.iter().map(|(label, _)| text_width(label)).max().unwrap_or(0) + 2;
        for (label, value) in entries {
            let mut line = vec![Seg::new(pad(&format!("{label}:"), label_width), dim())];
            line.extend(to_segs(value, &name_style, true));
            lines.push(line);
        }
    }
    if lines.is_empty() {
        lines.push(vec![Seg::new("(no details)", dim())]);
    }
    lines.iter().flat_map(|line| wrap(line, width)).map(PRow::Text).collect()
}

fn description_rows(app: &App, profile: &fchat::Profile, width: usize) -> Vec<PRow> {
    let name_style = |name: &str| app.name_style(name);
    let lines = if profile.description.trim().is_empty() {
        vec![vec![Seg::new("(no description)", dim())]]
    } else {
        split_lines(&to_segs(&profile.description, &name_style, true))
    };
    lines.iter().flat_map(|line| wrap(line, width)).map(PRow::Text).collect()
}

/// Breaks a text into lines of at most `width` cells, at word boundaries where possible.
fn chunks(text: &str, width: usize) -> Vec<String> {
    wrap(&[Seg::new(text, Style::default())], width)
        .iter()
        .map(|row| row.iter().map(|seg| seg.text.as_str()).collect::<String>().trim_end().to_string())
        .collect()
}

/// Kinks in columns side by side like in the Python version: Favorites, Yes, Maybe, No.
fn kink_rows(profile: &fchat::Profile, width: usize) -> Vec<PRow> {
    let per_band = if width >= 4 * 12 + 3 * PROFILE_GUTTER { 4 } else if width >= 2 * 12 + PROFILE_GUTTER { 2 } else { 1 };
    let column = ((width.saturating_sub(PROFILE_GUTTER * (per_band - 1))) / per_band).max(1);
    let columns: Vec<Vec<(String, Style)>> = KINK_TITLES
        .iter()
        .map(|(choice, title)| {
            let names = profile.kinks.iter().find(|(key, _)| key == choice).map(|(_, names)| names.as_slice()).unwrap_or(&[]);
            let mut lines = vec![(fit(&format!("{title} ({})", names.len()), column), heading())];
            for name in names {
                lines.extend(chunks(name, column).into_iter().map(|piece| (piece, Style::default())));
            }
            lines
        })
        .collect();
    let mut rows = Vec::new();
    for band in columns.chunks(per_band) {
        if !rows.is_empty() {
            rows.push(PRow::Text(Vec::new()));
        }
        let height = band.iter().map(Vec::len).max().unwrap_or(0);
        for index in 0..height {
            let mut segs = Vec::new();
            for (number, lines) in band.iter().enumerate() {
                if number > 0 {
                    segs.push(Seg::new(" ".repeat(PROFILE_GUTTER), Style::default()));
                }
                let (text, style) = lines.get(index).cloned().unwrap_or_default();
                let fill = column.saturating_sub(text_width(&text));
                segs.push(Seg::new(text, style));
                segs.push(Seg::new(" ".repeat(fill), Style::default()));
            }
            rows.push(PRow::Text(segs));
        }
    }
    rows
}

/// Thumbnails in a grid of up to five columns, each with its description below, centered.
/// Requests the images, so they load only while the tab is open.
fn image_rows(app: &mut App, profile: &fchat::Profile, width: usize) -> Vec<PRow> {
    let mut rows = Vec::new();
    if profile.images.is_empty() {
        rows.push(PRow::Text(vec![Seg::new("(no images)", dim())]));
        return rows;
    }
    if !app.images.enabled() {
        rows.push(PRow::Text(vec![Seg::new("Images are off; click a line to copy the link to the full image", dim())]));
        rows.push(PRow::Text(Vec::new()));
        for (number, (file, description)) in profile.images.iter().enumerate() {
            let text = if description.is_empty() { format!("{}. {file}", number + 1) } else { format!("{}. {description}", number + 1) };
            let line = vec![Seg::linked(text, crate::style::url_style(), full_image_url(file))];
            rows.extend(wrap(&line, width).into_iter().map(PRow::Text));
        }
        return rows;
    }
    rows.push(PRow::Text(vec![Seg::new("Click an image to copy the link to the full image", dim())]));
    rows.push(PRow::Text(Vec::new()));
    let (font_width, font_height) = app.images.font();
    let estimate = (u32::from(images::THUMB_ROWS) * u32::from(font_height)).div_ceil(u32::from(font_width.max(1))) as usize;
    let thumbs: Vec<(Key, String, String)> = profile.images.iter().map(|(file, description)| (Key::new(Kind::Thumb, file), full_image_url(file), description.clone())).collect();
    let widths: Vec<Option<usize>> = thumbs
        .iter()
        .map(|(key, _, _)| {
            app.images.request(key);
            app.images.size_cells(key).map(|size| size.0 as usize)
        })
        .collect();
    let widest = widths.iter().map(|width| width.unwrap_or(estimate)).max().unwrap_or(estimate).max(1);
    let per_row = ((width + PROFILE_GUTTER) / (widest + PROFILE_GUTTER)).clamp(1, images::THUMB_COLUMNS);
    let cell = (width.saturating_sub(PROFILE_GUTTER * (per_row - 1)) / per_row).max(1);
    for (band, (thumbs, widths)) in thumbs.chunks(per_row).zip(widths.chunks(per_row)).enumerate() {
        if band > 0 {
            rows.push(PRow::Text(Vec::new()));
        }
        let mut items = Vec::new();
        let mut slots = Vec::new();
        for (index, ((key, link, _), image_width)) in thumbs.iter().zip(widths).enumerate() {
            let start = index * (cell + PROFILE_GUTTER);
            let slot = match image_width {
                Some(image_width) => (start + cell.saturating_sub(*image_width) / 2, (*image_width).min(cell)),
                None => (start, cell),
            };
            items.push((key.clone(), Some(link.clone())));
            slots.push((slot.0 as u16, slot.1 as u16));
        }
        push_strip(&mut rows, items, images::THUMB_ROWS, slots);
        let texts: Vec<Vec<String>> = thumbs
            .iter()
            .map(|(_, _, description)| {
                let mut pieces = if description.is_empty() { Vec::new() } else { chunks(description, cell) };
                if pieces.len() > IMAGE_TEXT_ROWS {
                    pieces.truncate(IMAGE_TEXT_ROWS);
                    let last = pieces.last_mut().unwrap();
                    let mut cut = fit(last, cell.saturating_sub(3));
                    cut.push_str("...");
                    *last = cut;
                }
                pieces
            })
            .collect();
        let height = texts.iter().map(Vec::len).max().unwrap_or(0);
        for line in 0..height {
            let mut segs = Vec::new();
            for (index, pieces) in texts.iter().enumerate() {
                if index > 0 {
                    segs.push(Seg::new(" ".repeat(PROFILE_GUTTER), Style::default()));
                }
                let text = pieces.get(line).cloned().unwrap_or_default();
                let free = cell.saturating_sub(text_width(&text));
                segs.push(Seg::new(" ".repeat(free / 2), Style::default()));
                segs.push(Seg::new(text, dim()));
                segs.push(Seg::new(" ".repeat(free - free / 2), Style::default()));
            }
            rows.push(PRow::Text(segs));
        }
    }
    rows
}

/// Draws rows with their links and image strips, scrolled by `scroll`.
fn draw_rows(frame: &mut Frame, app: &mut App, area: Rect, rows: &[PRow], scroll: &mut usize) {
    let height = area.height as usize;
    *scroll = (*scroll).min(rows.len().saturating_sub(height));
    let mut shown = Vec::new();
    let mut strips: Vec<PendingStrip> = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(*scroll).take(height) {
        let y = area.y + (index - *scroll) as u16;
        match row {
            PRow::Text(segs) => {
                let mut x = 0usize;
                for seg in segs {
                    let w = text_width(&seg.text);
                    if let Some(link) = &seg.link {
                        app.regions.modal_links.push((rect(area.x + x as u16, y, w as u16, 1), link.clone()));
                    }
                    x += w;
                }
                shown.push(spans(segs));
            }
            PRow::Strip(items, strip_rows, slots) => {
                let complete = index + *strip_rows as usize <= *scroll + height;
                strips.push((y, items.clone(), *strip_rows, slots.clone(), complete));
                shown.push(Line::default());
            }
            PRow::Cont => shown.push(Line::default()),
        }
    }
    frame.render_widget(Paragraph::new(shown), area);
    for (y, items, strip_rows, slots, complete) in strips {
        let target = StripTarget { x: area.x, y, width: area.width, rows: strip_rows, draw: complete, chat: false, gap: 1, slots };
        draw_strip(frame, app, &items, &target);
    }
}

type StripItems = Vec<(Key, Option<String>)>;
/// A strip seen while laying out rows: y, images, rows, places, completely visible.
type PendingStrip = (u16, StripItems, u16, Vec<(u16, u16)>, bool);

/// The profile page: header with avatar and the main facts, tabs below.
fn draw_profile(frame: &mut Frame, app: &mut App, area: Rect, screen: &mut ProfileScreen) {
    let outer = centered(area, (area.width * 9 / 10).max(40), (area.height * 9 / 10).max(12));
    let inner = modal(frame, outer, &format!("Profile: {}", screen.character));
    let body = rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height.saturating_sub(1));

    let avatar = app.images.enabled().then(|| Key::new(Kind::Avatar, &screen.character));
    let avatar_width = match &avatar {
        Some(key) => {
            app.images.request(key);
            let (font_width, font_height) = app.images.font();
            let estimate = (u32::from(images::AVATAR_ROWS) * u32::from(font_height)).div_ceil(u32::from(font_width.max(1))) as u16;
            app.images.size_cells(key).map_or(estimate, |size| size.0)
        }
        None => 0,
    };
    let info_x = if avatar_width > 0 { body.x + avatar_width + 2 } else { body.x };
    let info_width = body.width.saturating_sub(info_x - body.x);
    let header: Vec<Vec<Seg>> = profile_header(app, screen).iter().flat_map(|line| wrap(line, info_width as usize)).collect();
    let header_rows = (header.len() as u16).max(if avatar.is_some() { images::AVATAR_ROWS } else { 0 }).min(body.height / 2);
    if let Some(key) = &avatar {
        let target = StripTarget { x: body.x, y: body.y, width: avatar_width.max(1), rows: images::AVATAR_ROWS, draw: header_rows >= images::AVATAR_ROWS, chat: false, gap: 0, slots: Vec::new() };
        draw_strip(frame, app, &[(key.clone(), None)], &target);
    }
    let mut header_scroll = 0;
    let header_lines: Vec<PRow> = header.into_iter().map(PRow::Text).collect();
    draw_rows(frame, app, rect(info_x, body.y, info_width, header_rows), &header_lines, &mut header_scroll);

    let tabs_y = body.y + header_rows + 1;
    let count = match &screen.result {
        Some(Ok(profile)) => format!("Images ({})", profile.images.len()),
        _ => "Images".to_string(),
    };
    let mut x = body.x;
    let mut tab_spans = Vec::new();
    let mut underline = Vec::new();
    screen.tab_hits.clear();
    for (tab, label) in PROFILE_TABS.iter().zip(["Info", "Description", "Kinks", count.as_str()]) {
        let text = format!(" {label} ");
        let width = text_width(&text) as u16;
        let active = *tab == screen.tab;
        screen.tab_hits.push((rect(x, tabs_y, width, 1), *tab));
        tab_spans.push(Span::styled(text, if active { color("focus").patch(bold()) } else { dim() }));
        tab_spans.push(Span::raw(" "));
        underline.push(Span::styled((if active { "\u{2501}" } else { "\u{2500}" }).repeat(width as usize), if active { color("focus") } else { dim() }));
        underline.push(Span::styled("\u{2500}", dim()));
        x += width + 1;
    }
    let rest = body.width.saturating_sub(x - body.x) as usize;
    underline.push(Span::styled("\u{2500}".repeat(rest), dim()));
    frame.render_widget(Paragraph::new(vec![Line::from(tab_spans), Line::from(underline)]), rect(body.x, tabs_y, body.width, 2));

    let content = rect(body.x, tabs_y + 3, body.width, body.height.saturating_sub(header_rows + 4));
    let width = content.width as usize;
    let rows = match &screen.result {
        Some(Ok(profile)) => {
            let profile = profile.clone();
            match screen.tab {
                ProfileTab::Info => info_rows(app, &profile, width),
                ProfileTab::Description => description_rows(app, &profile, width),
                ProfileTab::Kinks => kink_rows(&profile, width),
                ProfileTab::Images => image_rows(app, &profile, width),
            }
        }
        _ => Vec::new(),
    };
    draw_rows(frame, app, content, &rows, screen.scroll_mut());
    footer(frame, outer, "o: copy profile link, b: bookmark, f: friend request, left/right: tabs, up/down: scroll, esc: close, click names and links");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn text_of(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>()).collect()
    }

    fn app_with_input(text: &str) -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx);
        app.input.set_text(text);
        app
    }

    #[test]
    fn private_chats_show_who_is_online_after_the_name() {
        let mut app = app_with_input("");
        app.store.get_or_create("Zed", USER, None);
        app.store.get_or_create("Xi", USER, None);
        app.presence.set_online("Zed", "busy", "");
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 4)).unwrap();
        terminal.draw(|frame| draw_chat_list(frame, &mut app, frame.area())).unwrap();
        let rows: Vec<String> = (0..4).map(|y| (0..20).map(|x| terminal.backend().buffer()[(x, y)].symbol().to_string()).collect::<String>()).collect();
        assert!(rows.iter().any(|row| row.contains("@Zed \u{25cf}")), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("@Xi \u{25cb}")), "{rows:?}");
        let zed = rows.iter().position(|row| row.contains("@Zed")).unwrap() as u16;
        let dot = terminal.backend().buffer()[(6, zed)].style().fg;
        assert_eq!(dot, status_style("busy").fg);
    }

    #[test]
    fn the_settings_page_shows_the_version() {
        let mut settings = SettingsScreen::new(&Config::default());
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(220, 30)).unwrap();
        terminal.draw(|frame| {
            draw_settings(frame, frame.area(), &mut settings);
        })
        .unwrap();
        let shown: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(shown.contains(&format!("terminalchat {}", env!("CARGO_PKG_VERSION"))));
    }

    #[test]
    fn hints_list_matching_commands() {
        let lines = text_of(&hint_lines(&app_with_input("/ba")));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("/ban <name>"), "{lines:?}");
        assert!(lines[1].contains("/banlist"));
    }

    #[test]
    fn hints_resolve_aliases_after_a_space() {
        let lines = text_of(&hint_lines(&app_with_input("/profil bob")));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with(" /profile [name]"), "{lines:?}");
        assert!(hint_lines(&app_with_input("hello")).is_empty());
    }

    #[test]
    fn many_matches_are_cut() {
        let lines = text_of(&hint_lines(&app_with_input("/")));
        assert_eq!(lines.len(), MAX_HINTS);
        assert!(lines[MAX_HINTS - 1].contains("more, keep typing"));
    }

    #[test]
    fn the_settings_page_draws_in_small_and_large_terminals() {
        for (width, height) in [(40, 14), (120, 40)] {
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut screen = Screen::Settings(SettingsScreen::new(&Config::default()));
            for _ in 0..2 {
                let draw = |terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, screen: &mut Screen| {
                    let Screen::Settings(settings) = screen else { unreachable!() };
                    terminal.draw(|frame| {
                        draw_settings(frame, frame.area(), settings);
                    })
                    .unwrap();
                };
                draw(&mut terminal, &mut screen);
                screen.on_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Enter));
                draw(&mut terminal, &mut screen);
                screen.on_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Esc));
                screen.on_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::End));
                screen.on_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Tab));
            }
            let text: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("Settings"));
        }
    }

    #[test]
    fn the_settings_page_reacts_to_clicks() {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 34)).unwrap();
        let mut settings = SettingsScreen::new(&Config::default());
        let draw = |terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, settings: &mut SettingsScreen| {
            terminal.draw(|frame| {
                draw_settings(frame, frame.area(), settings);
            })
            .unwrap();
        };
        let click = |settings: &mut SettingsScreen, hit: Hit| {
            let (area, _) = *settings.hits.iter().find(|(_, found)| *found == hit).unwrap_or_else(|| panic!("{hit:?} not drawn"));
            settings.click(area.x, area.y)
        };
        draw(&mut terminal, &mut settings);
        click(&mut settings, Hit::Tab(Tab::Colors));
        assert_eq!(settings.tab, Tab::Colors);
        draw(&mut terminal, &mut settings);
        let url = crate::colors::entries().iter().position(|entry| entry.path == "url").unwrap();
        click(&mut settings, Hit::Row(url));
        assert_eq!(settings.current(), url);
        assert!(settings.edit.is_none(), "the first click only chooses");
        click(&mut settings, Hit::Row(url));
        draw(&mut terminal, &mut settings);
        click(&mut settings, Hit::Swatch(ColorPart::Color, 1));
        click(&mut settings, Hit::Modifier(0));
        let Some(SettingsEdit::Color(editor)) = &settings.edit else { panic!("no color editor") };
        assert_eq!(editor.input.text(), "bold underline #000000");
        click(&mut settings, Hit::Tab(Tab::General));
        assert!(settings.edit.is_none());
        draw(&mut terminal, &mut settings);
        let sidebar = crate::settings::SETTINGS.iter().position(|setting| setting.key == "show_sidebar").unwrap();
        click(&mut settings, Hit::Row(sidebar));
        let action = click(&mut settings, Hit::Row(sidebar));
        assert!(matches!(action, crate::screens::Action::Setting(key, value) if key == "show_sidebar" && value == "true"));
        click(&mut settings, Hit::Reset);
        assert!(settings.confirm_reset);
        draw(&mut terminal, &mut settings);
        assert!(matches!(click(&mut settings, Hit::Reset), crate::screens::Action::ResetTab(Tab::General)));
    }

    #[test]
    fn profile_thumbnails_sit_in_their_own_columns() {
        let mut app = app_with_input("");
        app.images.enable_for_test();
        let files = ["a.png", "b.png", "c.png", "d.png", "e.png", "f.png", "g.png"];
        for (index, file) in files.iter().enumerate() {
            if index != 3 {
                app.images.insert_for_test(Key::new(Kind::Thumb, file), 60 + 20 * index as u32, 100);
            }
        }
        let images = files.iter().map(|file| (file.to_string(), "A description that is far too long for one column of the grid".to_string())).collect();
        let profile = fchat::Profile { name: "X".into(), images, ..Default::default() };
        let mut screen = ProfileScreen::new("X");
        screen.result = Some(Ok(profile));
        screen.tab = ProfileTab::Images;
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 50)).unwrap();
        terminal
            .draw(|frame| {
                draw_profile(frame, &mut app, frame.area(), &mut screen);
            })
            .unwrap();
        let thumbs: Vec<Rect> = app.regions.modal_links.iter().filter(|(_, link)| link.ends_with(".png")).map(|(area, _)| *area).collect();
        assert_eq!(thumbs.len(), files.len());
        for (index, first) in thumbs.iter().enumerate() {
            for second in &thumbs[index + 1..] {
                assert!(!first.intersects(*second), "{first:?} overlaps {second:?}");
            }
        }
        let text: Vec<String> = (0..50).map(|y| (0..110).map(|x| terminal.backend().buffer().cell((x, y)).unwrap().symbol().to_string()).collect()).collect();
        assert!(text.iter().any(|line| line.contains("...")), "long descriptions are cut after two lines");
        assert!(!text.iter().any(|line| line.contains("colum n")), "lines break between words");
    }
}
