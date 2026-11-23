//! Drawing of the notes window.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use fchat::Folder;

use crate::app::App;
use crate::bbcode::{split_lines, to_segs, Seg};
use crate::input::InputBox;
use crate::notes::{Compose, Field, Hit, NotesScreen, Reading, FOLDERS};
use crate::rows::{text_width, wrap};
use crate::style::{bold, color, dim, reversed};
use crate::ui::{centered, draw_lines, footer, modal, rect, wrap_input};
use ratatui::layout::Rect;

const WHEN: usize = 16;
const NAME: usize = 22;
const LABEL: u16 = 8;

/// Cuts or pads text to a width in cells.
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

fn folder_label(screen: &NotesScreen, folder: Folder) -> String {
    let list = &screen.lists[FOLDERS.iter().position(|f| *f == folder).unwrap_or(0)];
    let name = match folder {
        Folder::Inbox => "Inbox",
        Folder::Sent => "Sent",
    };
    if list.loading && list.notes.is_empty() {
        format!(" {name} ")
    } else {
        format!(" {name} ({}) ", list.total)
    }
}

pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, screen: &mut NotesScreen) -> Option<(u16, u16)> {
    let outer = centered(area, (area.width * 9 / 10).max(40), (area.height * 9 / 10).max(12));
    let inner = modal(frame, outer, "Notes");
    let body = rect(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height.saturating_sub(1));
    screen.hits.clear();

    let mut x = body.x;
    let mut tabs = Vec::new();
    let mut underline = Vec::new();
    for folder in FOLDERS {
        let text = folder_label(screen, folder);
        let width = text_width(&text) as u16;
        let active = folder == screen.folder && screen.compose.is_none();
        screen.hits.push((rect(x, body.y, width, 1), Hit::Folder(folder)));
        tabs.push(Span::styled(text, if active { color("focus").patch(bold()) } else { dim() }));
        tabs.push(Span::raw(" "));
        underline.push(Span::styled((if active { "\u{2501}" } else { "\u{2500}" }).repeat(width as usize), if active { color("focus") } else { dim() }));
        underline.push(Span::styled("\u{2500}", dim()));
        x += width + 1;
    }
    if screen.compose.is_some() {
        tabs.push(Span::styled(" New note ", color("focus").patch(bold())));
    }
    underline.push(Span::styled("\u{2500}".repeat(body.width.saturating_sub(x - body.x) as usize), dim()));
    frame.render_widget(Paragraph::new(vec![Line::from(tabs), Line::from(underline)]), rect(body.x, body.y, body.width, 2));

    let (status, style) = prompt(screen);
    frame.render_widget(Paragraph::new(Span::styled(status, style)), rect(body.x, body.y + 2, body.width, 1));

    let content = rect(body.x, body.y + 3, body.width, body.height.saturating_sub(3));
    let mut cursor = None;
    let hint = if screen.compose.is_some() {
        let NotesScreen { compose, characters, hits, .. } = screen;
        cursor = draw_compose(frame, app, content, compose.as_mut().unwrap(), characters, hits);
        "tab: next field, left/right: sender, enter: send (in the message), ctrl+j: new line, ctrl+e: emoji, esc: cancel"
    } else if let Some(reading) = screen.reading.as_mut() {
        draw_reading(frame, app, content, reading);
        "r: reply, d: trash, up/down: scroll, esc: back, click links"
    } else {
        draw_list(frame, app, content, screen);
        "enter: read, r: reply, n: new, d: trash, left/right: folder, f5: reload, esc: close"
    };
    footer(frame, outer, hint);
    cursor
}

fn prompt(screen: &NotesScreen) -> (String, Style) {
    if screen.confirm_trash.is_some() {
        return ("Move this note to the trash? y/n".into(), color("warning").patch(bold()));
    }
    if screen.compose.as_ref().is_some_and(|compose| compose.confirm_discard) {
        return ("Discard this note? y/n".into(), color("warning").patch(bold()));
    }
    (screen.status.clone(), color("warning"))
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect, screen: &mut NotesScreen) {
    let list = screen.list();
    if list.notes.is_empty() {
        let text = if list.loading { "Loading..." } else { "(no notes)" };
        frame.render_widget(Paragraph::new(Span::styled(text, dim())), area);
        return;
    }
    let width = area.width as usize;
    let title_width = width.saturating_sub(WHEN + 2 * NAME + 3);
    let header = format!("{} {} {} {}", cell("Sent", WHEN), cell("From", NAME), cell("To", NAME), cell("Title", title_width));
    let mut lines = vec![Line::from(Span::styled(header, dim()))];
    let capacity = area.height.saturating_sub(1) as usize;
    let start = (list.selected + 1).saturating_sub(capacity);
    let mut hits = Vec::new();
    for (index, note) in list.notes.iter().enumerate().skip(start).take(capacity) {
        let selected = index == list.selected;
        let base = if selected { reversed() } else { Style::default() };
        let name = |name: &str| if selected { base } else { app.name_style(name) };
        hits.push((rect(area.x, area.y + 1 + (index - start) as u16, area.width, 1), Hit::Row(index)));
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", cell(&note.sent, WHEN)), if selected { base } else { dim() }),
            Span::styled(format!("{} ", cell(&note.source, NAME)), name(&note.source)),
            Span::styled(format!("{} ", cell(&note.dest, NAME)), name(&note.dest)),
            Span::styled(cell(&note.title, title_width), base),
        ]));
    }
    let more = list.notes.len() < list.total;
    if more && list.selected + 1 >= list.notes.len() && lines.len() <= capacity {
        lines.push(Line::from(Span::styled("Loading more...", dim())));
    }
    screen.hits.extend(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_reading(frame: &mut Frame, app: &mut App, area: Rect, reading: &mut Reading) {
    let header = &reading.header;
    let line = |label: &str, value: &str, style: Style| vec![Seg::new(format!("{label:<7}"), dim()), Seg::new(value, style)];
    let mut lines = vec![
        line("From", &header.source, app.name_style(&header.source)),
        line("To", &header.dest, app.name_style(&header.dest)),
        line("Sent", &header.sent, Style::default()),
        line("Title", &header.title, bold()),
        vec![Seg::new("\u{2500}".repeat(area.width as usize), dim())],
    ];
    match &reading.text {
        None => lines.push(vec![Seg::new("Loading...", dim())]),
        Some(Err(error)) => lines.push(vec![Seg::new(error, color("warning"))]),
        Some(Ok(text)) => {
            let name_style = |name: &str| app.name_style(name);
            lines.extend(split_lines(&to_segs(text, &name_style, true)));
        }
    }
    let width = area.width as usize;
    let lines: Vec<Vec<Seg>> = lines.iter().flat_map(|line| wrap(line, width)).collect();
    draw_lines(frame, app, area, &lines, &mut reading.scroll);
    crate::ui::selectable(frame, app, area);
}

/// One input row: the end of the text stays visible while typing.
fn single(frame: &mut Frame, area: Rect, input: &InputBox, focused: bool) -> Option<(u16, u16)> {
    crate::colors::fill(frame, area, "status_field");
    let width = area.width.saturating_sub(1) as usize;
    let chars: Vec<char> = input.text().chars().collect();
    let start = input.cursor().saturating_sub(width);
    let shown: String = chars.iter().skip(start).take(width).collect();
    frame.render_widget(Paragraph::new(shown), area);
    focused.then(|| (area.x + text_width(&chars[start..input.cursor()].iter().collect::<String>()) as u16, area.y))
}

fn draw_compose(frame: &mut Frame, app: &mut App, area: Rect, compose: &mut Compose, characters: &[String], hits: &mut Vec<(Rect, Hit)>) -> Option<(u16, u16)> {
    let field_x = area.x + LABEL;
    let field_width = area.width.saturating_sub(LABEL);
    let label = |frame: &mut Frame, row: u16, text: &str, field: Field| {
        let style = if compose.field == field { color("focus").patch(bold()) } else { dim() };
        frame.render_widget(Paragraph::new(Span::styled(text.to_string(), style)), rect(area.x, area.y + row, LABEL, 1));
    };
    label(frame, 0, "From", Field::From);
    label(frame, 1, "To", Field::To);
    label(frame, 2, "Title", Field::Title);
    label(frame, 4, "Message", Field::Message);
    let from = characters.get(compose.from).cloned().unwrap_or_default();
    let from_style = if compose.field == Field::From { reversed() } else { app.name_style(&from) };
    let from_line = Line::from(vec![Span::styled("\u{2039} ", dim()), Span::styled(from, from_style), Span::styled(format!(" \u{203a}   ({} characters)", characters.len()), dim())]);
    frame.render_widget(Paragraph::new(from_line), rect(field_x, area.y, field_width, 1));
    for (row, field) in [(0, Field::From), (1, Field::To), (2, Field::Title)] {
        hits.push((rect(area.x, area.y + row, area.width, 1), Hit::Field(field)));
    }
    let to = single(frame, rect(field_x, area.y + 1, field_width, 1), &compose.to, compose.field == Field::To);
    let title = single(frame, rect(field_x, area.y + 2, field_width, 1), &compose.title, compose.field == Field::Title);

    let message = rect(area.x, area.y + 5, area.width, area.height.saturating_sub(5));
    hits.push((message, Hit::Field(Field::Message)));
    crate::colors::fill(frame, message, "status_field");
    let (lines, (row, column)) = wrap_input(compose.message.text(), compose.message.cursor(), message.width.saturating_sub(1).max(1) as usize);
    let start = (row + 1).saturating_sub(message.height as usize);
    let shown: Vec<Line> = lines.iter().skip(start).take(message.height as usize).map(|line| Line::from(line.clone())).collect();
    frame.render_widget(Paragraph::new(shown), message);
    let count = format!(" {} ", compose.message.char_count());
    let count_width = text_width(&count) as u16;
    frame.render_widget(Paragraph::new(Span::styled(count, dim())), rect(area.x + area.width.saturating_sub(count_width), area.y + 4, count_width, 1));
    let message_cursor = row.checked_sub(start).filter(|_| compose.field == Field::Message).map(|row| (message.x + column as u16, message.y + row as u16));
    if compose.sending {
        return None;
    }
    to.or(title).or(message_cursor)
}


#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent};

    use super::*;
    use crate::config::Config;
    use crate::notes::Reply;


    fn shown(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect()
    }


    #[test]
    fn list_note_and_draft_draw_at_any_size() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(Config::default(), tx);
        for (width, height) in [(120, 40), (40, 12), (12, 5)] {
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            let mut screen = NotesScreen::new("Bo");
            screen.on_reply(Reply::Characters(vec!["Anna".into(), "Bo".into()]));
            let note = fchat::NoteHeader { id: 1, title: "A rather long title of a note".into(), source: "Zed".into(), dest: "Anna".into(), sent: "2y, 5mo, 3d ago".into() };
            screen.on_reply(Reply::List { folder: Folder::Inbox, offset: 0, notes: vec![note], total: 1 });
            let mut draw = |screen: &mut NotesScreen| {
                terminal.draw(|frame| {
                    draw(frame, &mut app, frame.area(), screen);
                })
                .unwrap();
                shown(&terminal)
            };
            let list = draw(&mut screen);
            screen.on_key(KeyEvent::from(KeyCode::Enter));
            screen.on_reply(Reply::Text(1, "Hi [b]there[/b]\nsecond line".into()));
            let note = draw(&mut screen);
            screen.on_key(KeyEvent::from(KeyCode::Char('r')));
            for c in "long text ".repeat(40).chars() {
                screen.on_key(KeyEvent::from(KeyCode::Char(c)));
            }
            let draft = draw(&mut screen);
            if width == 120 {
                assert!(list.contains("Inbox (1)") && list.contains("A rather long title"), "{list}");
                assert!(note.contains("there") && note.contains("second line"), "{note}");
                assert!(draft.contains("Re: A rather long") && draft.contains("Anna"), "{draft}");
            }
        }
    }
}
