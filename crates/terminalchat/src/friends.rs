//! The friends window (/friends): friendships of all characters of the account, friend requests in
//! both directions and bookmarks. The API calls run in tasks; their answers come back as
//! `AppEvent::Friends`.

use fchat::{Answer, FriendData};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::input::InputBox;
use crate::screens::Action;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Load,
    Bookmark(String, bool),
    Send { source: String, dest: String },
    Answer { id: u64, answer: Answer, name: String },
    Remove { own: String, other: String },
}

pub enum Reply {
    /// A fresh API ticket, for the next calls.
    Ticket(String),
    Data(FriendData),
    Done(String),
    Failed(String),
}

fn done_text(request: &Request) -> String {
    match request {
        Request::Load => String::new(),
        Request::Bookmark(name, true) => format!("Bookmarked {name}"),
        Request::Bookmark(name, false) => format!("Bookmark of {name} removed"),
        Request::Send { source, dest } => format!("Friend request sent from {source} to {dest}"),
        Request::Answer { answer: Answer::Accept, name, .. } => format!("{name} is now a friend"),
        Request::Answer { answer: Answer::Deny, name, .. } => format!("Request of {name} denied"),
        Request::Answer { answer: Answer::Cancel, name, .. } => format!("Request to {name} taken back"),
        Request::Remove { own, other } => format!("{other} is no longer a friend of {own}"),
    }
}

/// Runs a request, then loads all lists again. An expired ticket is renewed once.
pub fn spawn(tx: UnboundedSender<AppEvent>, account: String, password: String, ticket: Option<String>, own: Vec<String>, request: Request) {
    tokio::spawn(async move {
        let send = |reply: Reply| {
            let _ = tx.send(AppEvent::Friends(reply));
        };
        let renew = || async {
            let ticket = fchat::fetch_ticket(&account, &password).await?.ticket;
            let _ = tx.send(AppEvent::Friends(Reply::Ticket(ticket.clone())));
            Ok::<String, fchat::Error>(ticket)
        };
        let result = async {
            let mut ticket = match ticket {
                Some(ticket) => ticket,
                None => renew().await?,
            };
            let first = perform(&account, &ticket, &own, &request).await;
            let data = match first {
                Err(error) if error.to_string().to_lowercase().contains("ticket") => {
                    ticket = renew().await?;
                    perform(&account, &ticket, &own, &request).await?
                }
                other => other?,
            };
            Ok::<FriendData, fchat::Error>(data)
        }
        .await;
        match result {
            Ok(data) => {
                if request != Request::Load {
                    send(Reply::Done(done_text(&request)));
                }
                send(Reply::Data(data));
            }
            Err(error) => send(Reply::Failed(error.to_string())),
        }
    });
}

async fn perform(account: &str, ticket: &str, own: &[String], request: &Request) -> fchat::Result<FriendData> {
    match request {
        Request::Load => {}
        Request::Bookmark(name, add) => fchat::friends::bookmark(account, ticket, name, *add).await?,
        Request::Send { source, dest } => fchat::friends::send_request(account, ticket, source, dest).await?,
        Request::Answer { id, answer, .. } => fchat::friends::answer(account, ticket, *id, *answer).await?,
        Request::Remove { own: character, other } => fchat::friends::remove_friend(account, ticket, character, other).await?,
    }
    fchat::friends::load(account, ticket, own).await
}

// ------------------------------------------------------------------ screen

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Friends,
    Incoming,
    Outgoing,
    Bookmarks,
}

pub const TABS: [Tab; 4] = [Tab::Friends, Tab::Incoming, Tab::Outgoing, Tab::Bookmarks];

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Friends => "Friends",
            Tab::Incoming => "Requests to you",
            Tab::Outgoing => "Your requests",
            Tab::Bookmarks => "Bookmarks",
        }
    }

    pub fn index(self) -> usize {
        TABS.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

/// One row of a list: the other character, the own one (if any) and what `d` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub other: String,
    pub own: String,
    pub id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    From,
    To,
}

pub struct Compose {
    /// A bookmark needs only the name.
    pub bookmark: bool,
    pub from: usize,
    pub to: InputBox,
    pub field: Field,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Tab(Tab),
    Row(usize),
    Field(Field),
}

pub struct FriendsScreen {
    pub tab: Tab,
    pub data: Option<FriendData>,
    pub selected: [usize; 4],
    /// First shown display line per tab; it moves only when the selection leaves the view.
    pub top: [usize; 4],
    pub characters: Vec<String>,
    pub own: String,
    pub loading: bool,
    pub status: String,
    pub compose: Option<Compose>,
    /// A request waiting for y, with its question.
    pub confirm: Option<(Request, String)>,
    pub hits: Vec<(Rect, Hit)>,
    pub requests: Vec<Request>,
    /// Filters all tabs by name; `/` starts typing.
    pub search: InputBox,
    pub searching: bool,
}

fn is_ctrl(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

impl FriendsScreen {
    pub fn new(characters: &[String], own: &str, search: &str) -> FriendsScreen {
        let mut input = InputBox::new();
        input.history_enabled = false;
        input.set_text(search.trim());
        FriendsScreen {
            tab: Tab::Friends,
            data: None,
            selected: [0; 4],
            top: [0; 4],
            characters: characters.to_vec(),
            own: own.to_string(),
            loading: true,
            status: "Loading...".into(),
            compose: None,
            confirm: None,
            hits: Vec::new(),
            requests: vec![Request::Load],
            search: input,
            searching: false,
        }
    }

    /// The rows of a tab; all but Bookmarks grouped by the own character, the active one first.
    pub fn rows(&self, tab: Tab) -> Vec<Row> {
        let mut rows = self.raw_rows(tab);
        let search = self.search.text().trim().to_lowercase();
        if !search.is_empty() {
            rows.retain(|row| row.other.to_lowercase().contains(&search));
        }
        if tab != Tab::Bookmarks {
            rows.sort_by_key(|row| (!row.own.eq_ignore_ascii_case(&self.own), row.own.to_lowercase(), row.other.to_lowercase()));
        }
        rows
    }

    fn raw_rows(&self, tab: Tab) -> Vec<Row> {
        let Some(data) = &self.data else { return Vec::new() };
        match tab {
            Tab::Friends => data.friends.iter().map(|f| Row { other: f.other.clone(), own: f.own.clone(), id: 0 }).collect(),
            Tab::Incoming => data.incoming.iter().map(|r| Row { other: r.source.clone(), own: r.dest.clone(), id: r.id }).collect(),
            Tab::Outgoing => data.outgoing.iter().map(|r| Row { other: r.dest.clone(), own: r.source.clone(), id: r.id }).collect(),
            Tab::Bookmarks => data.bookmarks.iter().map(|name| Row { other: name.clone(), own: String::new(), id: 0 }).collect(),
        }
    }

    pub fn count(&self, tab: Tab) -> usize {
        self.rows(tab).len()
    }

    pub fn selected_row(&self) -> Option<Row> {
        self.rows(self.tab).into_iter().nth(self.selected[self.tab.index()])
    }

    pub fn selected_index(&self) -> usize {
        self.selected[self.tab.index()]
    }

    pub fn on_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Ticket(_) => {}
            Reply::Data(data) => {
                self.data = Some(data);
                self.loading = false;
                if self.status == "Loading..." {
                    self.status.clear();
                }
                for tab in TABS {
                    let last = self.count(tab).saturating_sub(1);
                    let selected = &mut self.selected[tab.index()];
                    *selected = (*selected).min(last);
                }
            }
            Reply::Done(text) => self.status = text,
            Reply::Failed(error) => {
                self.loading = false;
                self.status = error;
            }
        }
    }

    /// After the filter changed: selections back into the lists, views to the top.
    fn clamp(&mut self) {
        for tab in TABS {
            let last = self.count(tab).saturating_sub(1);
            self.selected[tab.index()] = self.selected[tab.index()].min(last);
            self.top[tab.index()] = 0;
        }
    }

    fn search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.search.set_text("");
                self.searching = false;
            }
            KeyCode::Enter | KeyCode::Down => self.searching = false,
            KeyCode::Backspace => self.search.backspace(),
            KeyCode::Delete => self.search.delete(),
            KeyCode::Left => self.search.left(),
            KeyCode::Right => self.search.right(),
            KeyCode::Home => self.search.home(),
            KeyCode::End => self.search.end(),
            KeyCode::Char('u') if is_ctrl(&key) => self.search.set_text(""),
            KeyCode::Char(c) if !is_ctrl(&key) => self.search.insert_char(c),
            _ => return,
        }
        self.clamp();
    }

    fn ask(&mut self, request: Request) {
        self.loading = true;
        self.status = "Working...".into();
        self.requests.push(request);
    }

    pub fn move_selection(&mut self, step: isize) {
        let last = self.count(self.tab).saturating_sub(1) as isize;
        let selected = &mut self.selected[self.tab.index()];
        *selected = (*selected as isize + step).clamp(0, last.max(0)) as usize;
    }

    fn step_tab(&mut self, step: isize) {
        let index = (self.tab.index() as isize + step).rem_euclid(TABS.len() as isize) as usize;
        self.tab = TABS[index];
    }

    pub fn compose_new(&mut self, bookmark: bool) {
        let from = self.characters.iter().position(|name| name.eq_ignore_ascii_case(&self.own)).unwrap_or(0);
        let mut to = InputBox::new();
        to.history_enabled = false;
        self.compose = Some(Compose { bookmark, from, to, field: Field::To });
    }

    /// What `d` does on the selected row, with the question to confirm it.
    fn removal(&self, row: &Row) -> (Request, String) {
        match self.tab {
            Tab::Friends => (Request::Remove { own: row.own.clone(), other: row.other.clone() }, format!("End the friendship of {} and {}? y/n", row.own, row.other)),
            Tab::Incoming => (Request::Answer { id: row.id, answer: Answer::Deny, name: row.other.clone() }, format!("Deny the request of {}? y/n", row.other)),
            Tab::Outgoing => (Request::Answer { id: row.id, answer: Answer::Cancel, name: row.other.clone() }, format!("Take back the request to {}? y/n", row.other)),
            Tab::Bookmarks => (Request::Bookmark(row.other.clone(), false), format!("Remove the bookmark of {}? y/n", row.other)),
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(compose) = self.compose.as_mut() {
            compose.to.insert_str(&text.replace('\n', " "));
        }
    }

    pub fn click(&mut self, column: u16, row: u16) -> Action {
        let hit = self.hits.iter().find(|(area, _)| column >= area.x && column < area.x + area.width && row >= area.y && row < area.y + area.height).map(|(_, hit)| *hit);
        match hit {
            Some(Hit::Tab(tab)) if self.compose.is_none() => self.tab = tab,
            Some(Hit::Row(index)) if self.compose.is_none() => {
                if self.selected_index() == index {
                    if let Some(row) = self.selected_row() {
                        return Action::Private(row.other);
                    }
                }
                self.selected[self.tab.index()] = index;
            }
            Some(Hit::Field(field)) => {
                let count = self.characters.len().max(1);
                if let Some(compose) = self.compose.as_mut() {
                    if field == Field::From && compose.field == Field::From {
                        compose.from = (compose.from + 1) % count;
                    }
                    compose.field = field;
                }
            }
            _ => {}
        }
        Action::None
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        if let Some((request, _)) = self.confirm.take() {
            if matches!(key.code, KeyCode::Char('y') | KeyCode::Char('j') | KeyCode::Enter) {
                self.ask(request);
            }
            return Action::None;
        }
        if self.compose.is_some() {
            self.compose_key(key);
            return Action::None;
        }
        if self.searching {
            self.search_key(key);
            return Action::None;
        }
        let row = self.selected_row();
        match key.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-15),
            KeyCode::PageDown => self.move_selection(15),
            KeyCode::Home => self.move_selection(isize::MIN / 2),
            KeyCode::End => self.move_selection(isize::MAX / 2),
            KeyCode::Left | KeyCode::BackTab => self.step_tab(-1),
            KeyCode::Right | KeyCode::Tab => self.step_tab(1),
            KeyCode::F(5) => self.ask(Request::Load),
            KeyCode::Char('n') => self.compose_new(self.tab == Tab::Bookmarks),
            KeyCode::Enter => {
                if let Some(row) = row {
                    return Action::Private(row.other);
                }
            }
            KeyCode::Char('p') => {
                if let Some(row) = row {
                    return Action::Profile(row.other);
                }
            }
            KeyCode::Char('a') if self.tab == Tab::Incoming => {
                if let Some(row) = row {
                    self.ask(Request::Answer { id: row.id, answer: Answer::Accept, name: row.other });
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(row) = row {
                    self.confirm = Some(self.removal(&row));
                }
            }
            KeyCode::Char('/') => self.searching = true,
            KeyCode::Esc if !self.search.text().is_empty() => {
                self.search.set_text("");
                self.clamp();
            }
            KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
            _ => {}
        }
        Action::None
    }

    fn compose_key(&mut self, key: KeyEvent) {
        let count = self.characters.len().max(1);
        let Some(compose) = self.compose.as_mut() else { return };
        match key.code {
            KeyCode::Esc => self.compose = None,
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down if !compose.bookmark => {
                compose.field = if compose.field == Field::From { Field::To } else { Field::From };
            }
            KeyCode::Left | KeyCode::Right if compose.field == Field::From => {
                let step = if key.code == KeyCode::Left { count - 1 } else { 1 };
                compose.from = (compose.from + step) % count;
            }
            KeyCode::Enter if compose.field == Field::From => compose.field = Field::To,
            KeyCode::Enter => {
                let dest = compose.to.text().trim().to_string();
                if dest.is_empty() {
                    self.status = "Enter a character name".into();
                    return;
                }
                let request = if compose.bookmark {
                    Request::Bookmark(dest, true)
                } else {
                    Request::Send { source: self.characters.get(compose.from).cloned().unwrap_or_else(|| self.own.clone()), dest }
                };
                self.compose = None;
                self.ask(request);
            }
            KeyCode::Backspace if compose.field == Field::To => compose.to.backspace(),
            KeyCode::Delete if compose.field == Field::To => compose.to.delete(),
            KeyCode::Left => compose.to.left(),
            KeyCode::Right => compose.to.right(),
            KeyCode::Home => compose.to.home(),
            KeyCode::End => compose.to.end(),
            KeyCode::Char('u') if is_ctrl(&key) => compose.to.set_text(""),
            KeyCode::Char(c) if !is_ctrl(&key) && compose.field == Field::To => compose.to.insert_char(c),
            _ => {}
        }
    }
}


#[cfg(test)]
mod tests {
    use fchat::{FriendRequest, Friendship};

    use super::*;


    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }


    fn screen() -> FriendsScreen {
        let mut screen = FriendsScreen::new(&["Anna".into(), "Bo".into()], "Bo", "");
        screen.requests.clear();
        screen.on_reply(Reply::Data(FriendData {
            friends: vec![Friendship { own: "Anna".into(), other: "Zed".into() }],
            incoming: vec![FriendRequest { id: 7, source: "Yan".into(), dest: "Bo".into() }],
            outgoing: vec![FriendRequest { id: 8, source: "Anna".into(), dest: "Xi".into() }],
            bookmarks: vec!["Abe".into()],
        }));
        screen
    }


    #[test]
    fn every_tab_removes_with_a_confirmation() {
        let mut screen = screen();
        let expected = [
            Request::Remove { own: "Anna".into(), other: "Zed".into() },
            Request::Answer { id: 7, answer: Answer::Deny, name: "Yan".into() },
            Request::Answer { id: 8, answer: Answer::Cancel, name: "Xi".into() },
            Request::Bookmark("Abe".into(), false),
        ];
        for request in expected {
            screen.on_key(key(KeyCode::Char('d')));
            screen.on_key(key(KeyCode::Char('n')));
            assert!(screen.requests.is_empty());
            screen.on_key(key(KeyCode::Char('d')));
            screen.on_key(key(KeyCode::Char('y')));
            assert_eq!(screen.requests.pop(), Some(request));
            screen.on_key(key(KeyCode::Right));
        }
        assert_eq!(screen.tab, Tab::Friends);
    }


    #[test]
    fn requests_are_accepted_and_sent_from_a_chosen_character() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Right));
        screen.on_key(key(KeyCode::Char('a')));
        assert_eq!(screen.requests.pop(), Some(Request::Answer { id: 7, answer: Answer::Accept, name: "Yan".into() }));
        screen.on_key(key(KeyCode::Char('n')));
        for c in "Wu".chars() {
            screen.on_key(key(KeyCode::Char(c)));
        }
        screen.on_key(key(KeyCode::Tab));
        screen.on_key(key(KeyCode::Right));
        screen.on_key(key(KeyCode::Enter));
        screen.on_key(key(KeyCode::Enter));
        assert_eq!(screen.requests.pop(), Some(Request::Send { source: "Anna".into(), dest: "Wu".into() }));
        assert!(matches!(screen.on_key(key(KeyCode::Enter)), Action::Private(name) if name == "Yan"));
        assert!(matches!(screen.on_key(key(KeyCode::Char('p'))), Action::Profile(name) if name == "Yan"));
    }


    #[test]
    fn rows_are_grouped_by_own_character_with_the_active_one_first() {
        let mut screen = screen();
        screen.data.as_mut().unwrap().friends = vec![
            Friendship { own: "Anna".into(), other: "Ab".into() },
            Friendship { own: "Bo".into(), other: "Zz".into() },
            Friendship { own: "Anna".into(), other: "Aa".into() },
        ];
        let order: Vec<(String, String)> = screen.rows(Tab::Friends).into_iter().map(|row| (row.own, row.other)).collect();
        let expected = [("Bo", "Zz"), ("Anna", "Aa"), ("Anna", "Ab")].map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(order, expected);
    }


    #[test]
    fn the_search_filters_every_tab_and_esc_clears_it() {
        let mut screen = screen();
        screen.on_key(key(KeyCode::Char('/')));
        for c in "ZE".chars() {
            screen.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(TABS.map(|tab| screen.count(tab)), [1, 0, 0, 0]);
        screen.on_key(key(KeyCode::Enter));
        assert!(matches!(screen.on_key(key(KeyCode::Enter)), Action::Private(name) if name == "Zed"));
        assert!(matches!(screen.on_key(key(KeyCode::Esc)), Action::None));
        assert_eq!(TABS.map(|tab| screen.count(tab)), [1, 1, 1, 1]);
        assert!(matches!(screen.on_key(key(KeyCode::Esc)), Action::Close));
    }


    #[test]
    fn a_bookmark_needs_only_a_name() {
        let mut screen = screen();
        screen.tab = Tab::Bookmarks;
        screen.on_key(key(KeyCode::Char('n')));
        for c in "Cy".chars() {
            screen.on_key(key(KeyCode::Char(c)));
        }
        screen.on_key(key(KeyCode::Enter));
        assert_eq!(screen.requests.pop(), Some(Request::Bookmark("Cy".into(), true)));
    }
}
