//! The notes window (ctrl+n, /note): the F-List notes of all characters of the account, read,
//! answer as any of them and write new ones. The website calls run in tasks; their answers come
//! back as `AppEvent::Notes`.

use fchat::{Folder, NoteHeader, NotesSession};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::input::InputBox;
use crate::screens::Action;

/// Notes loaded per call; more follow when the selection reaches the end.
pub const PAGE: usize = 50;
pub const FOLDERS: [Folder; 2] = [Folder::Inbox, Folder::Sent];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Log in, then load the first page of both folders.
    Open,
    List(Folder, usize),
    Read(u64),
    Send(Draft),
    Trash(Folder, u64),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub source: String,
    pub dest: String,
    pub title: String,
    pub message: String,
}

pub enum Reply {
    Session(NotesSession),
    Characters(Vec<String>),
    List { folder: Folder, offset: usize, notes: Vec<NoteHeader>, total: usize },
    Text(u64, String),
    Sent(String),
    Trashed(Folder, u64),
    Failed(Request, String),
}

/// Runs one request in a task. A request with an old session that fails is tried once more
/// after a new login, because website sessions expire.
pub fn spawn(tx: UnboundedSender<AppEvent>, session: Option<NotesSession>, account: String, password: String, request: Request) {
    tokio::spawn(async move {
        let result = async {
            let fresh = session.is_none();
            let session = match session {
                Some(session) => session,
                None => login(&tx, &account, &password).await?,
            };
            match perform(&tx, &session, &request).await {
                Err(_) if !fresh => {
                    let session = login(&tx, &account, &password).await?;
                    perform(&tx, &session, &request).await
                }
                other => other,
            }
        }
        .await;
        if let Err(error) = result {
            let _ = tx.send(AppEvent::Notes(Reply::Failed(request, error.to_string())));
        }
    });
}

async fn login(tx: &UnboundedSender<AppEvent>, account: &str, password: &str) -> fchat::Result<NotesSession> {
    let session = NotesSession::login(account, password).await?;
    let _ = tx.send(AppEvent::Notes(Reply::Session(session.clone())));
    Ok(session)
}

async fn perform(tx: &UnboundedSender<AppEvent>, session: &NotesSession, request: &Request) -> fchat::Result<()> {
    let send = |reply: Reply| {
        let _ = tx.send(AppEvent::Notes(reply));
    };
    match request {
        Request::Open => {
            send(Reply::Characters(session.characters.iter().map(|(name, _)| name.clone()).collect()));
            for folder in FOLDERS {
                let (notes, total) = session.list(folder, 0, PAGE).await?;
                send(Reply::List { folder, offset: 0, notes, total });
            }
        }
        Request::List(folder, offset) => {
            let (notes, total) = session.list(*folder, *offset, PAGE).await?;
            send(Reply::List { folder: *folder, offset: *offset, notes, total });
        }
        Request::Read(id) => send(Reply::Text(*id, session.text(*id).await?)),
        Request::Send(draft) => {
            session.send(&draft.source, &draft.dest, &draft.title, &draft.message).await?;
            send(Reply::Sent(draft.dest.clone()));
        }
        Request::Trash(folder, id) => {
            session.trash(&[*id]).await?;
            send(Reply::Trashed(*folder, *id));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ screen

#[derive(Default)]
pub struct NoteList {
    pub notes: Vec<NoteHeader>,
    pub total: usize,
    pub selected: usize,
    pub loading: bool,
}

pub struct Reading {
    pub folder: Folder,
    pub header: NoteHeader,
    pub text: Option<Result<String, String>>,
    pub scroll: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    From,
    To,
    Title,
    Message,
}

const FIELDS: [Field; 4] = [Field::From, Field::To, Field::Title, Field::Message];

pub struct Compose {
    pub from: usize,
    pub to: InputBox,
    pub title: InputBox,
    pub message: InputBox,
    pub field: Field,
    pub sending: bool,
    /// Esc on a draft with text asks first.
    pub confirm_discard: bool,
}

impl Compose {
    fn input(&mut self) -> Option<&mut InputBox> {
        match self.field {
            Field::From => None,
            Field::To => Some(&mut self.to),
            Field::Title => Some(&mut self.title),
            Field::Message => Some(&mut self.message),
        }
    }

    fn step(&mut self, step: isize) {
        let position = FIELDS.iter().position(|field| *field == self.field).unwrap_or(0) as isize;
        self.field = FIELDS[(position + step).rem_euclid(FIELDS.len() as isize) as usize];
    }
}

fn input(text: &str) -> InputBox {
    let mut input = InputBox::new();
    input.history_enabled = false;
    input.set_text(text);
    input
}

/// What a click hits; filled when drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    Folder(Folder),
    Row(usize),
    Field(Field),
}

pub struct NotesScreen {
    pub folder: Folder,
    pub lists: [NoteList; 2],
    /// The characters of the account, empty until logged in.
    pub characters: Vec<String>,
    /// The chat character, preselected as sender of new notes.
    pub own: String,
    pub reading: Option<Reading>,
    pub compose: Option<Compose>,
    pub confirm_trash: Option<(Folder, u64)>,
    pub status: String,
    pub hits: Vec<(Rect, Hit)>,
    /// Requests the app has to start, taken after each key.
    pub requests: Vec<Request>,
}

fn index(folder: Folder) -> usize {
    match folder {
        Folder::Inbox => 0,
        Folder::Sent => 1,
    }
}

fn is_ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

impl NotesScreen {
    pub fn new(own: &str) -> NotesScreen {
        let mut lists = [NoteList::default(), NoteList::default()];
        for list in &mut lists {
            list.loading = true;
        }
        NotesScreen {
            folder: Folder::Inbox,
            lists,
            characters: Vec::new(),
            own: own.to_string(),
            reading: None,
            compose: None,
            confirm_trash: None,
            status: "Logging in on f-list.net...".into(),
            hits: Vec::new(),
            requests: vec![Request::Open],
        }
    }

    pub fn list(&self) -> &NoteList {
        &self.lists[index(self.folder)]
    }

    fn list_mut(&mut self) -> &mut NoteList {
        &mut self.lists[index(self.folder)]
    }

    pub fn selected(&self) -> Option<&NoteHeader> {
        let list = self.list();
        list.notes.get(list.selected)
    }

    pub fn on_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Session(_) => {}
            Reply::Characters(names) => {
                self.characters = names;
                self.status.clear();
            }
            Reply::List { folder, offset, notes, total } => {
                let list = &mut self.lists[index(folder)];
                list.loading = false;
                list.total = total;
                if offset == 0 {
                    list.notes = notes;
                } else if offset == list.notes.len() {
                    list.notes.extend(notes);
                }
                list.selected = list.selected.min(list.notes.len().saturating_sub(1));
            }
            Reply::Text(id, text) => {
                if let Some(reading) = self.reading.as_mut().filter(|reading| reading.header.id == id) {
                    reading.text = Some(Ok(text));
                }
            }
            Reply::Sent(dest) => {
                self.compose = None;
                self.status = format!("Note sent to {dest}");
                self.reload(Folder::Sent);
            }
            Reply::Trashed(folder, id) => {
                let list = &mut self.lists[index(folder)];
                if let Some(position) = list.notes.iter().position(|note| note.id == id) {
                    list.notes.remove(position);
                    list.total = list.total.saturating_sub(1);
                    list.selected = list.selected.min(list.notes.len().saturating_sub(1));
                }
                if self.reading.as_ref().is_some_and(|reading| reading.header.id == id) {
                    self.reading = None;
                }
                self.status = "Moved to the trash".into();
            }
            Reply::Failed(request, error) => {
                self.status = error.clone();
                match request {
                    Request::Open => {
                        for list in &mut self.lists {
                            list.loading = false;
                        }
                    }
                    Request::List(folder, _) => self.lists[index(folder)].loading = false,
                    Request::Read(id) => {
                        if let Some(reading) = self.reading.as_mut().filter(|reading| reading.header.id == id) {
                            reading.text = Some(Err(error));
                        }
                    }
                    Request::Send(_) => {
                        if let Some(compose) = self.compose.as_mut() {
                            compose.sending = false;
                        }
                    }
                    Request::Trash(..) => {}
                }
            }
        }
    }

    fn reload(&mut self, folder: Folder) {
        self.lists[index(folder)].loading = true;
        self.requests.push(Request::List(folder, 0));
    }

    fn switch_folder(&mut self) {
        self.folder = if self.folder == Folder::Inbox { Folder::Sent } else { Folder::Inbox };
    }

    pub fn move_selection(&mut self, step: isize) {
        let list = self.list_mut();
        let last = list.notes.len().saturating_sub(1);
        list.selected = (list.selected as isize + step).clamp(0, last as isize) as usize;
        let more = !list.loading && list.notes.len() < list.total && list.selected + 1 >= list.notes.len();
        if more {
            list.loading = true;
            let offset = list.notes.len();
            let folder = self.folder;
            self.requests.push(Request::List(folder, offset));
        }
    }

    pub fn scroll(&mut self, rows: isize) {
        match (&mut self.reading, &self.compose) {
            (_, Some(_)) => {}
            (Some(reading), None) => reading.scroll = reading.scroll.saturating_add_signed(rows),
            (None, None) => self.move_selection(rows),
        }
    }

    fn open_selected(&mut self) {
        let Some(header) = self.selected().cloned() else { return };
        self.requests.push(Request::Read(header.id));
        self.reading = Some(Reading { folder: self.folder, header, text: None, scroll: 0 });
    }

    fn own_name(&self, name: &str) -> Option<usize> {
        self.characters.iter().position(|character| character.eq_ignore_ascii_case(name))
    }

    /// A new note; the sender is the chat character if it belongs to the account.
    pub fn compose_new(&mut self, to: &str) {
        if self.characters.is_empty() {
            self.status = "Not logged in on f-list.net yet".into();
            return;
        }
        let from = self.own_name(&self.own).unwrap_or(0);
        let field = if to.is_empty() { Field::To } else { Field::Title };
        self.compose = Some(Compose { from, to: input(to), title: input(""), message: input(""), field, sending: false, confirm_discard: false });
    }

    /// An answer: sent by the own character of the note to the other one.
    fn compose_reply(&mut self, header: &NoteHeader) {
        if self.characters.is_empty() {
            self.status = "Not logged in on f-list.net yet".into();
            return;
        }
        let (from, to) = match self.own_name(&header.dest) {
            Some(from) => (from, header.source.clone()),
            None => (self.own_name(&header.source).unwrap_or(0), header.dest.clone()),
        };
        let title = if header.title.to_lowercase().starts_with("re:") { header.title.clone() } else { format!("Re: {}", header.title) };
        self.compose = Some(Compose { from, to: input(&to), title: input(&title), message: input(""), field: Field::Message, sending: false, confirm_discard: false });
    }

    fn reply_target(&self) -> Option<NoteHeader> {
        match &self.reading {
            Some(reading) => Some(reading.header.clone()),
            None => self.selected().cloned(),
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(compose) = self.compose.as_mut() {
            let multi = compose.field == Field::Message;
            if let Some(input) = compose.input() {
                input.insert_str(&if multi { text.to_string() } else { text.replace('\n', " ") });
            }
        }
    }

    pub fn click(&mut self, column: u16, row: u16) {
        let hit = self.hits.iter().find(|(area, _)| column >= area.x && column < area.x + area.width && row >= area.y && row < area.y + area.height).map(|(_, hit)| *hit);
        match hit {
            Some(Hit::Folder(folder)) if self.compose.is_none() => {
                self.folder = folder;
                self.reading = None;
            }
            Some(Hit::Row(row)) if self.compose.is_none() && self.reading.is_none() => {
                if self.list().selected == row {
                    self.open_selected();
                } else {
                    let step = row as isize - self.list().selected as isize;
                    self.move_selection(step);
                }
            }
            Some(Hit::Field(field)) => {
                if let Some(compose) = self.compose.as_mut() {
                    if field == Field::From && compose.field == Field::From && !self.characters.is_empty() {
                        compose.from = (compose.from + 1) % self.characters.len();
                    }
                    compose.field = field;
                }
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if let Some((folder, id)) = self.confirm_trash {
            self.confirm_trash = None;
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('j') | KeyCode::Enter) {
                self.requests.push(Request::Trash(folder, id));
            }
            return Action::None;
        }
        if self.compose.is_some() {
            if key.code == KeyCode::Char('e') && is_ctrl(&key) && self.compose.as_ref().is_some_and(|c| c.field != Field::From && !c.sending) {
                return Action::OpenPicker(crate::screens::PickerKind::Emoji);
            }
            self.compose_key(key);
            return Action::None;
        }
        match key.code {
            KeyCode::Char('n') => self.compose_new(""),
            KeyCode::Char('r') => {
                if let Some(header) = self.reply_target() {
                    self.compose_reply(&header);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let target = match &self.reading {
                    Some(reading) => Some((reading.folder, reading.header.id)),
                    None => self.selected().map(|note| (self.folder, note.id)),
                };
                if target.is_some() {
                    self.confirm_trash = target;
                }
            }
            KeyCode::F(5) => {
                for folder in FOLDERS {
                    self.reload(folder);
                }
            }
            _ if self.reading.is_some() => return self.reading_key(key),
            _ => return self.list_key(key),
        }
        Action::None
    }

    fn list_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-15),
            KeyCode::PageDown => self.move_selection(15),
            KeyCode::Home => self.move_selection(isize::MIN / 2),
            KeyCode::End => self.move_selection(isize::MAX / 2),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab => self.switch_folder(),
            KeyCode::Enter => self.open_selected(),
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            _ => {}
        }
        Action::None
    }

    fn reading_key(&mut self, key: KeyEvent) -> Action {
        let Some(reading) = self.reading.as_mut() else { return Action::None };
        match key.code {
            KeyCode::Up => reading.scroll = reading.scroll.saturating_sub(1),
            KeyCode::Down => reading.scroll += 1,
            KeyCode::PageUp => reading.scroll = reading.scroll.saturating_sub(15),
            KeyCode::PageDown => reading.scroll += 15,
            KeyCode::Home => reading.scroll = 0,
            KeyCode::End => reading.scroll = usize::MAX / 2,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.reading = None,
            _ => {}
        }
        Action::None
    }

    fn compose_key(&mut self, key: KeyEvent) {
        let count = self.characters.len().max(1);
        let Some(compose) = self.compose.as_mut() else { return };
        if compose.sending {
            return;
        }
        if compose.confirm_discard {
            compose.confirm_discard = false;
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('j')) {
                self.compose = None;
            }
            return;
        }
        let newline = matches!(key.code, KeyCode::Enter) && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT)
            || matches!(key.code, KeyCode::Char('j')) && is_ctrl(&key);
        match key.code {
            _ if newline => {
                if compose.field == Field::Message {
                    compose.message.newline();
                }
            }
            KeyCode::Esc => {
                if compose.message.text().trim().is_empty() {
                    self.compose = None;
                } else {
                    compose.confirm_discard = true;
                }
            }
            KeyCode::Tab => compose.step(1),
            KeyCode::BackTab => compose.step(-1),
            KeyCode::Enter if compose.field != Field::Message => compose.step(1),
            KeyCode::Enter => self.send(),
            KeyCode::Left | KeyCode::Right if compose.field == Field::From => {
                let step = if key.code == KeyCode::Left { count - 1 } else { 1 };
                compose.from = (compose.from + step) % count;
            }
            KeyCode::Up if compose.field != Field::Message => compose.step(-1),
            KeyCode::Down if compose.field != Field::Message => compose.step(1),
            KeyCode::Up => {
                if !compose.message.move_up() {
                    compose.step(-1);
                }
            }
            KeyCode::Down => {
                compose.message.move_down();
            }
            _ => {
                let Some(input) = compose.input() else { return };
                match key.code {
                    KeyCode::Backspace => input.backspace(),
                    KeyCode::Delete => input.delete(),
                    KeyCode::Left => input.left(),
                    KeyCode::Right => input.right(),
                    KeyCode::Home => input.home(),
                    KeyCode::End => input.end(),
                    KeyCode::Char('u') if is_ctrl(&key) => input.set_text(""),
                    KeyCode::Char('a') if is_ctrl(&key) => input.home(),
                    KeyCode::Char(c) if !is_ctrl(&key) => input.insert_char(c),
                    _ => {}
                }
            }
        }
    }

    fn send(&mut self) {
        let Some(compose) = self.compose.as_mut() else { return };
        let draft = Draft {
            source: self.characters.get(compose.from).cloned().unwrap_or_default(),
            dest: compose.to.text().trim().to_string(),
            title: compose.title.text().trim().to_string(),
            message: compose.message.text().trim().to_string(),
        };
        let missing = [(&draft.dest, "a recipient", Field::To), (&draft.title, "a title", Field::Title), (&draft.message, "a message", Field::Message)];
        if let Some((_, what, field)) = missing.iter().find(|(value, _, _)| value.is_empty()) {
            self.status = format!("The note needs {what}");
            compose.field = *field;
            return;
        }
        compose.sending = true;
        self.status = format!("Sending to {}...", draft.dest);
        self.requests.push(Request::Send(draft));
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    fn header(id: u64, source: &str, dest: &str, title: &str) -> NoteHeader {
        NoteHeader { id, title: title.into(), source: source.into(), dest: dest.into(), sent: "1d ago".into() }
    }


    fn screen() -> NotesScreen {
        let mut screen = NotesScreen::new("Bo");
        screen.requests.clear();
        screen.on_reply(Reply::Characters(vec!["Anna".into(), "Bo".into()]));
        let notes = vec![header(1, "Zed", "Anna", "Hello"), header(2, "Yan", "Bo", "Re: Hi")];
        screen.on_reply(Reply::List { folder: Folder::Inbox, offset: 0, notes, total: 3 });
        screen
    }


    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }


    fn type_text(screen: &mut NotesScreen, text: &str) {
        for c in text.chars() {
            screen.on_key(key(KeyCode::Char(c)));
        }
    }


    #[test]
    fn a_reply_goes_from_the_receiving_character_back_to_the_sender() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Char('r')));
        let compose = screen.compose.as_ref().unwrap();
        assert_eq!((screen.characters[compose.from].as_str(), compose.to.text(), compose.title.text()), ("Anna", "Zed", "Re: Hello"));
        screen.compose = None;
        screen.on_key(key(KeyCode::Down));
        screen.on_key(key(KeyCode::Char('r')));
        let compose = screen.compose.as_ref().unwrap();
        assert_eq!((screen.characters[compose.from].as_str(), compose.to.text(), compose.title.text()), ("Bo", "Yan", "Re: Hi"));
    }


    #[test]
    fn sending_needs_all_fields_and_then_asks_the_app() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Char('n')));
        assert_eq!(screen.characters[screen.compose.as_ref().unwrap().from], "Bo");
        type_text(&mut screen, "Zed");
        screen.on_key(key(KeyCode::Enter));
        screen.on_key(key(KeyCode::Enter));
        screen.on_key(key(KeyCode::Enter));
        assert!(screen.status.contains("title"));
        type_text(&mut screen, "Hey");
        screen.on_key(key(KeyCode::Enter));
        type_text(&mut screen, "line one");
        screen.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        type_text(&mut screen, "two");
        screen.on_key(key(KeyCode::Enter));
        let draft = Draft { source: "Bo".into(), dest: "Zed".into(), title: "Hey".into(), message: "line one\ntwo".into() };
        assert_eq!(screen.requests, vec![Request::Send(draft)]);
        screen.on_reply(Reply::Sent("Zed".into()));
        assert!(screen.compose.is_none());
        assert_eq!(screen.requests.last(), Some(&Request::List(Folder::Sent, 0)));
    }


    #[test]
    fn the_from_field_cycles_through_the_characters() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Char('n')));
        screen.on_key(key(KeyCode::BackTab));
        screen.on_key(key(KeyCode::Right));
        assert_eq!(screen.compose.as_ref().unwrap().from, 0);
        screen.on_key(key(KeyCode::Left));
        assert_eq!(screen.compose.as_ref().unwrap().from, 1);
    }


    #[test]
    fn the_end_of_the_list_loads_more_and_trash_asks_first() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Down));
        assert_eq!(screen.requests, vec![Request::List(Folder::Inbox, 2)]);
        screen.on_reply(Reply::List { folder: Folder::Inbox, offset: 2, notes: vec![header(3, "X", "Bo", "Third")], total: 3 });
        assert_eq!(screen.list().notes.len(), 3);
        screen.on_key(key(KeyCode::Char('d')));
        screen.on_key(key(KeyCode::Char('n')));
        assert_eq!(screen.requests.len(), 1);
        screen.on_key(key(KeyCode::Char('d')));
        screen.on_key(key(KeyCode::Char('y')));
        assert_eq!(screen.requests.last(), Some(&Request::Trash(Folder::Inbox, 2)));
        screen.on_reply(Reply::Trashed(Folder::Inbox, 2));
        assert_eq!((screen.list().notes.len(), screen.list().total), (2, 2));
    }


    #[test]
    fn opening_a_note_asks_for_its_text() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Enter));
        assert_eq!(screen.requests, vec![Request::Read(1)]);
        screen.on_reply(Reply::Text(1, "[b]hi[/b]".into()));
        assert_eq!(screen.reading.as_ref().unwrap().text, Some(Ok("[b]hi[/b]".into())));
        screen.on_key(key(KeyCode::Esc));
        assert!(screen.reading.is_none());
        assert!(matches!(screen.on_key(key(KeyCode::Esc)), Action::Close));
    }
}
