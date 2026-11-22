//! Drawing of the friends window.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::friends::{Field, FriendsScreen, Hit, Tab, TABS};
use crate::rows::text_width;
use crate::style::{bold, color, dim, reversed, status_style};
use crate::ui::{centered, footer, modal, rect};

const NAME: usize = 24;
const LABEL: u16 = 8;

fn cell(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = text_width(&c.to_string());
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str(&" ".repeat(width - used));
    out
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, screen: &mut FriendsScreen) -> Option<(u16, u16)> {
    let outer = centered(area, (area.width * 9 / 10).max(40), (area.height * 9 / 10).max(12));
    let inner = modal(frame, outer, "Friends");
    let body = rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height.saturating_sub(1));
    screen.hits.clear();

    let mut x = body.x;
    let mut tabs = Vec::new();
    let mut underline = Vec::new();
    for tab in TABS {
        let text = if screen.data.is_some() { format!(" {} ({}) ", tab.label(), screen.count(tab)) } else { format!(" {} ", tab.label()) };
        let width = text_width(&text) as u16;
        let active = tab == screen.tab;
        screen.hits.push((rect(x, body.y, width, 1), Hit::Tab(tab)));
        tabs.push(Span::styled(text, if active { color("focus").patch(bold()) } else { dim() }));
        tabs.push(Span::raw(" "));
        underline.push(Span::styled((if active { "\u{2501}" } else { "\u{2500}" }).repeat(width as usize), if active { color("focus") } else { dim() }));
        underline.push(Span::styled("\u{2500}", dim()));
        x += width + 1;
    }
    underline.push(Span::styled("\u{2500}".repeat(body.width.saturating_sub(x.saturating_sub(body.x)) as usize), dim()));
    frame.render_widget(Paragraph::new(vec![Line::from(tabs), Line::from(underline)]), rect(body.x, body.y, body.width, 2));

    let mut cursor = None;
    let status_row = rect(body.x, body.y + 2, body.width, 1);
    if let Some((_, question)) = &screen.confirm {
        frame.render_widget(Paragraph::new(Span::styled(question.clone(), color("warning").patch(bold()))), status_row);
    } else if screen.searching || !screen.search.text().is_empty() {
        let label = "Search: ";
        let text = screen.search.text().to_string();
        let mut spans = vec![Span::styled(label, if screen.searching { color("focus").patch(bold()) } else { dim() }), Span::raw(text)];
        if !screen.status.is_empty() && !screen.searching {
            spans.push(Span::styled(format!("   {}", screen.status), color("warning")));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), status_row);
        if screen.searching {
            let before: String = screen.search.text().chars().take(screen.search.cursor()).collect();
            cursor = Some((status_row.x + (label.len() + text_width(&before)) as u16, status_row.y));
        }
    } else {
        frame.render_widget(Paragraph::new(Span::styled(screen.status.clone(), color("warning"))), status_row);
    }

    let content = rect(body.x, body.y + 3, body.width, body.height.saturating_sub(3));
    let (cursor, hint) = if screen.compose.is_some() {
        let cursor = draw_compose(frame, app, content, screen);
        let hint = if screen.compose.as_ref().is_some_and(|c| c.bookmark) { "enter: bookmark, esc: cancel" } else { "tab: field, left/right: sender, enter: send request, esc: cancel" };
        (cursor, hint)
    } else if screen.searching {
        draw_list(frame, app, content, screen);
        (cursor, "type a name, enter: done, esc: clear the search")
    } else {
        draw_list(frame, app, content, screen);
        let hint = match screen.tab {
            Tab::Friends => "/: search, enter: chat, p: profile, n: friend request, d: end friendship, left/right: tabs, f5: reload, esc: close",
            Tab::Incoming => "/: search, a: accept, d: deny, enter: chat, p: profile, left/right: tabs, f5: reload, esc: close",
            Tab::Outgoing => "/: search, d: take back, n: new request, enter: chat, p: profile, left/right: tabs, esc: close",
            Tab::Bookmarks => "/: search, n: add, d: remove, enter: chat, p: profile, left/right: tabs, f5: reload, esc: close",
        };
        (None, hint)
    };
    footer(frame, outer, hint);
    cursor
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect, screen: &mut FriendsScreen) {
    let rows = screen.rows(screen.tab);
    if rows.is_empty() {
        let text = if screen.loading { "Loading..." } else { "(none)" };
        frame.render_widget(Paragraph::new(Span::styled(text, dim())), area);
        return;
    }
    let grouped = screen.tab != Tab::Bookmarks;
    // Display lines: a heading before each own character, then its rows (None = heading).
    let mut display: Vec<Option<usize>> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let new_group = index == 0 || !rows[index - 1].own.eq_ignore_ascii_case(&row.own);
        if grouped && new_group {
            display.push(None);
        }
        display.push(Some(index));
    }
    let selected = screen.selected_index();
    let selected_line = display.iter().position(|line| *line == Some(selected)).unwrap_or(0);
    let capacity = (area.height as usize).max(1);
    let mut start = screen.top[screen.tab.index()].min(display.len().saturating_sub(capacity));
    if selected_line < start {
        start = selected_line;
        // the heading of the group stays visible when scrolling up to its first entry
        if start > 0 && display[start - 1].is_none() {
            start -= 1;
        }
    } else if selected_line >= start + capacity {
        start = selected_line + 1 - capacity;
    }
    screen.top[screen.tab.index()] = start;
    let mut lines = Vec::new();
    for (line_index, line) in display.iter().enumerate().skip(start).take(capacity) {
        let y = area.y + (line_index - start) as u16;
        let Some(index) = *line else {
            let own = &rows[display.get(line_index + 1).copied().flatten().unwrap_or(0)].own;
            let count = rows.iter().filter(|row| row.own.eq_ignore_ascii_case(own)).count();
            lines.push(Line::from(vec![
                Span::styled("\u{25be} ", dim()),
                Span::styled(own.clone(), app.name_style(own).patch(bold())),
                Span::styled(format!(" ({count})"), dim()),
            ]));
            continue;
        };
        let row = &rows[index];
        let chosen = index == selected;
        let base = if chosen { reversed() } else { Style::default() };
        let name = |name: &str| if chosen { base } else { app.name_style(name) };
        let indent = if grouped { "    " } else { "" };
        let mut spans = vec![Span::raw(indent), Span::styled(format!("{} ", cell(&row.other, NAME)), name(&row.other))];
        let status = match app.presence.status_of(&row.other) {
            Some((status, message)) if message.is_empty() => Span::styled(status.clone(), if chosen { base } else { status_style(&status) }),
            Some((status, message)) => Span::styled(format!("{status}: {message}"), if chosen { base } else { status_style(&status) }),
            None => Span::styled("offline", if chosen { base } else { dim() }),
        };
        spans.push(status);
        screen.hits.push((rect(area.x, y, area.width, 1), Hit::Row(index)));
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_compose(frame: &mut Frame, app: &mut App, area: Rect, screen: &mut FriendsScreen) -> Option<(u16, u16)> {
    let FriendsScreen { compose, characters, hits, .. } = screen;
    let compose = compose.as_mut()?;
    let field_x = area.x + LABEL;
    let field_width = area.width.saturating_sub(LABEL);
    let label_style = |field: Field| if compose.field == field { color("focus").patch(bold()) } else { dim() };
    let mut to_row = area.y;
    if !compose.bookmark {
        frame.render_widget(Paragraph::new(Span::styled("From", label_style(Field::From))), rect(area.x, area.y, LABEL, 1));
        let from = characters.get(compose.from).cloned().unwrap_or_default();
        let from_style = if compose.field == Field::From { reversed() } else { app.name_style(&from) };
        let line = Line::from(vec![Span::styled("\u{2039} ", dim()), Span::styled(from, from_style), Span::styled(" \u{203a}", dim())]);
        frame.render_widget(Paragraph::new(line), rect(field_x, area.y, field_width, 1));
        hits.push((rect(area.x, area.y, area.width, 1), Hit::Field(Field::From)));
        to_row += 1;
    }
    let label = if compose.bookmark { "Name" } else { "To" };
    frame.render_widget(Paragraph::new(Span::styled(label, label_style(Field::To))), rect(area.x, to_row, LABEL, 1));
    let field = rect(field_x, to_row, field_width, 1);
    hits.push((rect(area.x, to_row, area.width, 1), Hit::Field(Field::To)));
    crate::colors::fill(frame, field, "status_field");
    let width = field.width.saturating_sub(1) as usize;
    let chars: Vec<char> = compose.to.text().chars().collect();
    let start = compose.to.cursor().saturating_sub(width);
    let shown: String = chars.iter().skip(start).take(width).collect();
    frame.render_widget(Paragraph::new(shown), field);
    (compose.field == Field::To).then(|| (field.x + text_width(&chars[start..compose.to.cursor()].iter().collect::<String>()) as u16, field.y))
}


#[cfg(test)]
mod tests {
    use fchat::{FriendData, FriendRequest, Friendship};
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    use super::*;
    use crate::config::Config;
    use crate::friends::Reply;


    #[test]
    fn the_view_moves_only_when_the_selection_leaves_it() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
        let mut screen = FriendsScreen::new(&["Anna".into()], "Anna", "");
        let friends = (0..40).map(|n| Friendship { own: "Anna".into(), other: format!("F{n:02}") }).collect();
        screen.on_reply(Reply::Data(FriendData { friends, ..FriendData::default() }));
        let mut draw = |screen: &mut FriendsScreen| {
            terminal.draw(|frame| {
                draw(frame, &mut app, frame.area(), screen);
            })
            .unwrap();
        };
        screen.on_key(KeyEvent::from(KeyCode::End));
        draw(&mut screen);
        let top = screen.top[0];
        assert!(top > 0);
        screen.on_key(KeyEvent::from(KeyCode::Up));
        screen.on_key(KeyEvent::from(KeyCode::Up));
        draw(&mut screen);
        assert_eq!(screen.top[0], top);
        screen.on_key(KeyEvent::from(KeyCode::Home));
        draw(&mut screen);
        assert_eq!(screen.top[0], 0);
    }


    #[test]
    fn every_tab_and_the_form_draw_at_any_size() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx);
        for (width, height) in [(120, 40), (40, 12), (12, 5)] {
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut screen = FriendsScreen::new(&["Anna".into()], "Anna", "");
            screen.on_reply(Reply::Data(FriendData {
                friends: vec![Friendship { own: "Anna".into(), other: "Zed".into() }],
                incoming: vec![FriendRequest { id: 7, source: "Yan".into(), dest: "Anna".into() }],
                outgoing: Vec::new(),
                bookmarks: vec!["Abe".into()],
            }));
            let mut shown = String::new();
            for _ in TABS {
                terminal.draw(|frame| {
                    draw(frame, &mut app, frame.area(), &mut screen);
                })
                .unwrap();
                shown.push_str(&terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>());
                screen.on_key(KeyEvent::from(KeyCode::Right));
            }
            screen.on_key(KeyEvent::from(KeyCode::Char('n')));
            terminal.draw(|frame| {
                draw(frame, &mut app, frame.area(), &mut screen);
            })
            .unwrap();
            if width == 120 {
                assert!(shown.contains("Zed") && shown.contains("Yan") && shown.contains("Abe") && shown.contains("(none)"), "{shown}");
                assert!(shown.contains("\u{25be} Anna (1)"), "{shown}");
            }
        }
    }
}
