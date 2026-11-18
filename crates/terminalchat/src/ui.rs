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
use crate::screens::Screen;
use crate::store::{CHANNEL, CONSOLE, USER};
use crate::style::{bold, dim, fg, on, reversed, status_style, status_symbol};

const LIST_WIDTH: u16 = 26;
const SIDEBAR_WIDTH: u16 = 26;
const MAX_INPUT_ROWS: usize = 8;

fn spans(segs: &[Seg]) -> Line<'static> {
    Line::from(segs.iter().map(|seg| Span::styled(seg.text.clone(), seg.style)).collect::<Vec<_>>())
}

fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect { x, y, width, height }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    rect(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    app.regions.links.clear();
    app.regions.modal_links.clear();
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
    highlight_selection(frame, app);
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
    app.images.end_frame();
}

// ------------------------------------------------------------------ title

fn draw_title(frame: &mut Frame, app: &mut App, area: Rect) {
    app.regions.title = area;
    let online = if app.online { fg(crate::style::rgb(0x5fd75f)) } else { fg(crate::style::rgb(0xd75f5f)) };
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
        while text_width(&label) > max.saturating_sub(2) {
            label.pop();
        }
        let mut style = if chat.kind == USER { app.name_style(&chat.key) } else { Style::default() };
        if chat.unread {
            style = style.patch(bold());
            label.push_str(" \u{25cf}");
        }
        if app.active_chat.as_deref() == Some(id.as_str()) {
            style = style.patch(reversed());
            let pad = max.saturating_sub(text_width(&label));
            label.push_str(&" ".repeat(pad));
        }
        app.regions.chats.push((rect(inner.x, inner.y + (row - start) as u16, inner.width, 1), id.clone()));
        lines.push(Line::from(Span::styled(label, style)));
    }
    let border = if app.focus == Focus::List { fg(crate::style::rgb(0x5f87d7)) } else { dim() };
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
    let border = if app.focus == Focus::Sidebar { fg(crate::style::rgb(0x5f87d7)) } else { dim() };
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
            Span::styled(marker.to_string(), dim()),
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
        fg(crate::style::rgb(0xd7af5f))
    } else {
        Style::default()
    };
    let block = Block::default().borders(Borders::ALL).border_style(border).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
    let area = app.regions.chat;
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
    let bar = on(crate::style::rgb(0x1f2d4a));
    let key_style = bar.patch(fg(crate::style::rgb(0xffb000))).patch(bold());
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
        let color = match toast.severity {
            Severity::Information => crate::style::rgb(0x5f87d7),
            Severity::Warning => crate::style::rgb(0xd7af5f),
            Severity::Error => crate::style::rgb(0xd75f5f),
        };
        let toast_area = rect(area.x + area.width - width - 1, y, width, height);
        let mut block = Block::default().borders(Borders::ALL).border_style(fg(color));
        if !toast.title.is_empty() {
            block = block.title(Span::styled(format!(" {} ", toast.title), bold()));
        }
        frame.render_widget(Clear, toast_area);
        frame.render_widget(Paragraph::new(lines.iter().map(|row| spans(row)).collect::<Vec<_>>()).block(block), toast_area);
        y += height;
    }
}

// ------------------------------------------------------------------ screens

fn draw_lines(frame: &mut Frame, app: &mut App, area: Rect, lines: &[Vec<Seg>], scroll: &mut usize) {
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

fn modal(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).border_style(fg(crate::style::rgb(0x5f87d7))).title(Span::styled(format!(" {title} "), bold()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn footer(frame: &mut Frame, area: Rect, text: &str) {
    let line = rect(area.x + 2, area.y + area.height - 1, area.width.saturating_sub(4), 1);
    frame.render_widget(Paragraph::new(Span::styled(format!(" {text} "), dim())), line);
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
                Line::from(Span::styled(login.message.clone(), fg(crate::style::rgb(0xd7af5f)))),
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
        Screen::Profile(profile) => {
            let outer = centered(area, (area.width * 8 / 10).max(40), (area.height * 9 / 10).max(10));
            let inner = modal(frame, outer, &format!("Profile: {}", profile.character));
            let lines = match &profile.result {
                None => vec![PItem::Text(vec![Seg::new("Loading...", dim())])],
                Some(Err(error)) => vec![PItem::Text(vec![Seg::new(error.clone(), fg(crate::style::rgb(0xd75f5f)))])],
                Some(Ok(data)) => profile_lines(app, data),
            };
            let body = rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height);
            let avatar = app.images.enabled().then(|| Key::new(Kind::Avatar, &profile.character));
            draw_profile(frame, app, body, &lines, avatar, &mut profile.scroll);
            footer(frame, outer, "up/down/page: scroll, esc: close, click an image: copy its link");
        }
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
            let columns = 60usize;
            let mut lines: Vec<String> = vec![String::new()];
            let mut used = 0usize;
            let mut cursor_at = (0usize, 0usize);
            let cursor_index = edit.input.cursor();
            for (index, c) in edit.input.text().chars().enumerate() {
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
            if cursor_index >= edit.input.char_count() {
                if used >= columns {
                    lines.push(String::new());
                    used = 0;
                }
                cursor_at = (lines.len() - 1, used);
            }
            let outer = centered(area, 64, (lines.len() as u16 + 4).min(area.height));
            let inner = modal(frame, outer, &edit.title.clone());
            for (row, line) in lines.iter().enumerate() {
                frame.render_widget(Paragraph::new(line.clone()), rect(inner.x + 1, inner.y + 1 + row as u16, inner.width.saturating_sub(2), 1));
            }
            let count = format!("{}/{}", edit.input.char_count(), edit.max);
            frame.render_widget(Paragraph::new(Span::styled(count, dim())), rect(inner.x + 1, inner.y + 1 + lines.len() as u16, inner.width.saturating_sub(2), 1));
            cursor = Some((inner.x + 1 + cursor_at.1 as u16, inner.y + 1 + cursor_at.0 as u16));
            footer(frame, outer, "enter: save, ctrl+enter / ctrl+j: new line, esc: cancel, ctrl+u: clear");
        }
    }
    if let Some((x, y)) = cursor {
        frame.set_cursor_position((x, y));
    }
    app.screens.insert(index, screen);
}

// ------------------------------------------------------------------ profile text

fn format_time(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|time| time.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into())
}

pub enum PItem {
    Text(Vec<Seg>),
    /// Thumbnails: (image, link to the full image, description).
    Thumbs(Vec<(Key, String, String)>),
}

type StripItems = Vec<(Key, Option<String>)>;
type ProfileStrip = (u16, StripItems, u16, bool);

enum PRow {
    Text(Vec<Seg>),
    Strip(Vec<(Key, Option<String>)>, u16),
    Cont,
}

pub fn profile_lines(app: &App, profile: &fchat::Profile) -> Vec<PItem> {
    let mut lines: Vec<Vec<Seg>> = Vec::new();
    let name_style = |name: &str| app.name_style(name);
    lines.push(vec![Seg::new(profile.name.clone(), app.name_style(&profile.name).patch(bold()))]);
    if !profile.title.is_empty() {
        lines.push(vec![Seg::new(profile.title.clone(), dim())]);
    }
    lines.push(vec![Seg::new(
        format!("Views {}  \u{b7}  Created {}  \u{b7}  Updated {}", profile.views, format_time(profile.created), format_time(profile.updated)),
        dim(),
    )]);
    lines.push(vec![Seg::linked(crate::bbcode::profile_url(&profile.name), crate::style::url_style(), crate::bbcode::profile_url(&profile.name))]);
    lines.push(Vec::new());
    lines.extend(split_lines(&to_segs(&profile.description, &name_style, true)));
    for (group, items) in &profile.infotags {
        lines.push(Vec::new());
        lines.push(vec![Seg::new(group.clone(), bold())]);
        for (label, value) in items {
            lines.push(vec![Seg::new(format!("  {label}: "), dim()), Seg::new(value.clone(), Style::default())]);
        }
    }
    for (choice, names) in &profile.kinks {
        if names.is_empty() {
            continue;
        }
        lines.push(Vec::new());
        let mut label = choice.clone();
        if let Some(first) = label.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        lines.push(vec![Seg::new(format!("Kinks \u{2014} {label} ({})", names.len()), bold())]);
        lines.push(vec![Seg::new(names.join(", "), Style::default())]);
    }
    let mut items: Vec<PItem> = lines.into_iter().map(PItem::Text).collect();
    if !profile.images.is_empty() {
        items.push(PItem::Text(Vec::new()));
        items.push(PItem::Text(vec![Seg::new(format!("Images ({})", profile.images.len()), bold())]));
        if app.images.enabled() {
            let thumbs = profile.images.iter().map(|(file, description)| (Key::new(Kind::Thumb, file), full_image_url(file), description.clone())).collect();
            items.push(PItem::Thumbs(thumbs));
        } else {
            for (_, description) in &profile.images {
                if !description.is_empty() {
                    items.push(PItem::Text(vec![Seg::new(format!("  {description}"), dim())]));
                }
            }
        }
    }
    items
}

fn push_strip(rows: &mut Vec<PRow>, items: Vec<(Key, Option<String>)>, height: u16) {
    rows.push(PRow::Strip(items, height));
    for _ in 1..height {
        rows.push(PRow::Cont);
    }
}

fn expand_profile(app: &mut App, items: &[PItem], avatar: Option<Key>, width: usize) -> Vec<PRow> {
    let mut rows: Vec<PRow> = Vec::new();
    if let Some(key) = avatar {
        push_strip(&mut rows, vec![(key, None)], images::AVATAR_ROWS);
        rows.push(PRow::Text(Vec::new()));
    }
    let (font_width, font_height) = app.images.font();
    let estimate = (u32::from(images::THUMB_ROWS) * u32::from(font_height)).div_ceil(u32::from(font_width.max(1))) as usize;
    for item in items {
        match item {
            PItem::Text(line) => rows.extend(wrap(line, width).into_iter().map(PRow::Text)),
            PItem::Thumbs(thumbs) => {
                let mut group: Vec<(Key, Option<String>)> = Vec::new();
                let mut notes: Vec<String> = Vec::new();
                let mut used = 0usize;
                let flush = |rows: &mut Vec<PRow>, group: &mut Vec<(Key, Option<String>)>, notes: &mut Vec<String>, used: &mut usize| {
                    if group.is_empty() {
                        return;
                    }
                    push_strip(rows, std::mem::take(group), images::THUMB_ROWS);
                    rows.extend(notes.drain(..).map(|note| PRow::Text(vec![Seg::new(note, dim())])));
                    *used = 0;
                };
                for (number, (key, link, description)) in thumbs.iter().enumerate() {
                    app.images.request(key);
                    let columns = app.images.size_cells(key).map_or(estimate, |size| size.0 as usize);
                    if !group.is_empty() && (used + columns > width || group.len() >= images::THUMB_COLUMNS) {
                        flush(&mut rows, &mut group, &mut notes, &mut used);
                    }
                    group.push((key.clone(), Some(link.clone())));
                    if !description.is_empty() {
                        notes.push(format!("  {}. {description}", number + 1));
                    }
                    used += columns + 1;
                }
                flush(&mut rows, &mut group, &mut notes, &mut used);
            }
        }
    }
    rows
}

fn draw_profile(frame: &mut Frame, app: &mut App, area: Rect, items: &[PItem], avatar: Option<Key>, scroll: &mut usize) {
    let rows = expand_profile(app, items, avatar, area.width as usize);
    let height = area.height as usize;
    *scroll = (*scroll).min(rows.len().saturating_sub(height));
    let mut shown = Vec::new();
    let mut strips: Vec<ProfileStrip> = Vec::new();
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
            PRow::Strip(items, strip_rows) => {
                let complete = index + *strip_rows as usize <= *scroll + height;
                strips.push((y, items.clone(), *strip_rows, complete));
                shown.push(Line::default());
            }
            PRow::Cont => shown.push(Line::default()),
        }
    }
    frame.render_widget(Paragraph::new(shown), area);
    for (y, items, strip_rows, complete) in strips {
        let target = StripTarget { x: area.x, y, width: area.width, rows: strip_rows, draw: complete, chat: false, gap: 1, slots: Vec::new() };
        draw_strip(frame, app, &items, &target);
    }
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
}
