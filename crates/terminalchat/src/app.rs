//! The application state and the client logic: connection, server events, chats.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use fchat::opcode;
use fchat::{Connection, Identity, LoginResult};
use ratatui::crossterm::event::Event as TermEvent;
use ratatui::layout::Rect;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use crate::bbcode::{Seg, CHANNEL_LINK, SPOILER_LINK, USER_LINK};
use crate::config::Config;
use crate::images::{Frames, ImageStore, Key};
use crate::input::InputBox;
use crate::pager::{EntryKey, Pager, Source};
use crate::paths;
use crate::presence::Presence;
use crate::sanitize::{clean_data, html_unescape, is_safe_url};
use crate::screens::{CharacterScreen, InfoScreen, LoginScreen, Screen};
use crate::store::{make_id, ChatStore, AD_PREFIX, CHANNEL, CONSOLE, USER};
use crate::stores::{AdDrafts, AdSettings, GenderStore, OwnStatus, Pins};
use crate::style::gender_style;

pub const TITLE: &str = "Terminal Chat";
pub const RECONNECT_DELAYS: [u64; 6] = [5, 15, 30, 60, 120, 300];
const TICKET_MAX_AGE: Duration = Duration::from_secs(25 * 60);
const LIST_REFRESH: Duration = Duration::from_secs(300);
const GENDER_SAVE: Duration = Duration::from_secs(30);
const TOAST_SECONDS: u64 = 6;
const NO_RECONNECT_ERRORS: [i64; 2] = [9, 31];

pub enum TicketPurpose {
    /// Start of the program or character switch: continue with the connection.
    Startup,
    Reconnect,
    /// Result of the login screen.
    Login { account: String, password: String, remember: bool },
}

pub enum AppEvent {
    Term(TermEvent),
    Tick,
    Server(u64, fchat::Event),
    Ticket(TicketPurpose, fchat::Result<LoginResult>),
    Connected(u64, fchat::Result<Connection>),
    Profile(String, Result<fchat::Profile, String>),
    Image(Key, Option<Frames>),
    /// Whether a tmux client is attached (animations pause while detached).
    Attached(bool),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Information,
    Warning,
    Error,
}

pub struct Toast {
    pub title: String,
    pub text: String,
    pub severity: Severity,
    pub expires: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputMode {
    Chat,
    Ads,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Input,
    List,
    Chat,
    Sidebar,
}

/// A mouse selection in the chat view, in screen coordinates (column, row).
#[derive(Clone, Copy, Debug)]
pub struct Selection {
    pub start: (u16, u16),
    pub end: (u16, u16),
}

impl Selection {
    /// (first, last) in reading order.
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if (self.start.1, self.start.0) <= (self.end.1, self.end.0) {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SidebarTab {
    Friends,
    Channel,
}

pub struct Reconnect {
    pub attempt: usize,
    pub due: Instant,
    pub in_flight: bool,
}

/// Clickable areas of the last drawn frame.
#[derive(Default)]
pub struct Regions {
    pub links: Vec<(Rect, String, Option<EntryKey>)>,
    pub modal_links: Vec<(Rect, String)>,
    pub chats: Vec<(Rect, String)>,
    pub names: Vec<(Rect, String)>,
    pub modes: Vec<(Rect, InputMode)>,
    pub sidebar_tabs: Vec<(Rect, SidebarTab)>,
    pub footer: Vec<(Rect, char)>,
    pub chat: Rect,
    /// Plain text of the chat rows on screen, from the top of `chat`.
    pub chat_text: Vec<String>,
    pub title: Rect,
}

pub struct App {
    pub config: Config,
    pub tx: UnboundedSender<AppEvent>,
    pub should_quit: bool,
    pub osc_queue: Vec<String>,

    pub conn: Option<Connection>,
    pub generation: u64,
    pub online: bool,
    pub sub_title: String,
    pub character: String,
    pub characters: Vec<String>,
    pub ticket: String,
    pub ticket_time: Option<Instant>,
    pub bookmarks: Vec<String>,
    pub friend_pairs: Option<Vec<(String, String)>>,
    pub reconnect: Option<Reconnect>,
    pub startup_notes: Vec<String>,

    pub store: ChatStore,
    pub pins: Pins,
    pub ads: AdSettings,
    pub ad_drafts: AdDrafts,
    pub genders: GenderStore,
    pub own_status: OwnStatus,
    pub presence: Presence,
    pub variables: HashMap<String, Value>,
    pub last_ad: HashMap<String, Instant>,
    pub descriptions: HashMap<String, String>,
    pub last_error: Option<i64>,
    pub joining: HashSet<String>,
    pub pending_open: HashSet<String>,
    pub pending_create: HashSet<String>,
    pub joined: BTreeMap<String, String>,
    pub official: BTreeMap<String, String>,
    pub open_private: BTreeMap<String, String>,

    pub active_chat: Option<String>,
    pub input: InputBox,
    pub input_mode: InputMode,
    pub ad_channel: Option<String>,
    pub chat_draft: String,
    pub pager: Pager,
    pub images: ImageStore,
    pub screens: Vec<Screen>,
    pub toasts: Vec<Toast>,
    pub show_sidebar: bool,
    pub sidebar_tab: SidebarTab,
    pub title_expanded: bool,
    pub regions: Regions,
    pub focus: Focus,
    pub clipboard: String,
    pub selection: Option<Selection>,
    pub press: Option<(u16, u16)>,
    pub sidebar_cursor: usize,

    last_list_refresh: Option<Instant>,
    last_gender_save: Instant,
    ticks: u64,
    pub minimize_requested: bool,
}

fn text_of<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or("")
}

impl App {
    pub fn new(config: Config, tx: UnboundedSender<AppEvent>) -> App {
        let data_dir = config.data_dir();
        let character = config.character.clone();
        let chats_dir = data_dir.join("chats");
        let store = ChatStore::new(&character, &chats_dir);
        let base = store.base.clone();
        let show_sidebar = config.show_sidebar;
        let show_ads = config.show_ads;
        let mut app = App {
            images: ImageStore::new(tx.clone()),
            tx,
            should_quit: false,
            osc_queue: Vec::new(),
            conn: None,
            generation: 0,
            online: false,
            sub_title: "offline".into(),
            character,
            characters: Vec::new(),
            ticket: String::new(),
            ticket_time: None,
            bookmarks: Vec::new(),
            friend_pairs: None,
            reconnect: None,
            startup_notes: Vec::new(),
            pins: Pins::load(base.join("pins.json")),
            ads: AdSettings::load(base.join("ads.json"), show_ads),
            ad_drafts: AdDrafts::load(base.join("ad_drafts.json")),
            genders: GenderStore::load(chats_dir.join("genders.json")),
            own_status: OwnStatus::load(base.join("status.json")),
            store,
            presence: Presence::default(),
            variables: default_variables(),
            last_ad: HashMap::new(),
            descriptions: HashMap::new(),
            last_error: None,
            joining: HashSet::new(),
            pending_open: HashSet::new(),
            pending_create: HashSet::new(),
            joined: BTreeMap::new(),
            official: BTreeMap::new(),
            open_private: BTreeMap::new(),
            active_chat: None,
            input: InputBox::new(),
            input_mode: InputMode::Chat,
            ad_channel: None,
            chat_draft: String::new(),
            pager: Pager::default(),
            screens: Vec::new(),
            toasts: Vec::new(),
            show_sidebar,
            sidebar_tab: SidebarTab::Friends,
            title_expanded: false,
            regions: Regions::default(),
            focus: Focus::Input,
            clipboard: String::new(),
            selection: None,
            press: None,
            sidebar_cursor: 0,
            last_list_refresh: None,
            last_gender_save: Instant::now(),
            ticks: 0,
            minimize_requested: false,
            config,
        };
        app.store.get_or_create(CONSOLE, CONSOLE_KIND, Some("Console"));
        app.pager.live = true;
        app
    }

    // ---------------------------------------------------------------- notifications

    pub fn notify(&mut self, text: impl Into<String>, severity: Severity) {
        self.notify_titled("", text, severity, TOAST_SECONDS);
    }

    pub fn notify_titled(&mut self, title: &str, text: impl Into<String>, severity: Severity, seconds: u64) {
        self.toasts.push(Toast {
            title: title.into(),
            text: text.into(),
            severity,
            expires: Instant::now() + Duration::from_secs(seconds),
        });
        if self.toasts.len() > 6 {
            self.toasts.remove(0);
        }
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.notify(text, Severity::Warning);
    }

    pub fn copy_to_clipboard(&mut self, text: &str) {
        use base64::Engine;
        self.clipboard = text.to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(text);
        self.osc_queue.push(format!("\x1b]52;c;{encoded}\x07"));
    }

    /// The text under the mouse selection.
    pub fn selected_text(&self) -> String {
        let Some(selection) = &self.selection else { return String::new() };
        let ((c0, r0), (c1, r1)) = selection.ordered();
        let area = self.regions.chat;
        let mut lines = Vec::new();
        for y in r0..=r1 {
            let Some(line) = self.regions.chat_text.get(y.saturating_sub(area.y) as usize) else { continue };
            let from = if y == r0 { c0.saturating_sub(area.x) } else { 0 } as usize;
            let to = if y == r1 { c1.saturating_sub(area.x) as usize } else { usize::MAX };
            let mut column = 0usize;
            let mut text = String::new();
            for c in line.chars() {
                let width = crate::rows::char_width(c).max(1);
                if column + width > from && column <= to {
                    text.push(c);
                }
                column += width;
            }
            lines.push(text.trim_end().to_string());
        }
        lines.join("\n")
    }

    // ---------------------------------------------------------------- start

    /// Called once the event loop runs.
    pub fn start(&mut self) {
        if let Some(first) = self.first_chat_id() {
            self.open_chat(&first);
        }
        self.startup_notes.push(format!("Config: {} | Data: {}", self.config.path.display(), self.config.data_dir().display()));
        if self.config.is_complete() {
            self.begin_session();
        } else {
            self.show_login("");
        }
    }

    pub fn show_login(&mut self, message: &str) {
        let screen = LoginScreen::new(&self.config.account, self.config.remember_password, message);
        self.screens.push(Screen::Login(screen));
    }

    pub fn choose_character(&mut self, preferred: &str) {
        let preferred = if preferred.is_empty() { self.character.clone() } else { preferred.to_string() };
        self.screens.push(Screen::Character(CharacterScreen::new(self.characters.clone(), &preferred)));
    }

    fn ticket_is_fresh(&self) -> bool {
        !self.ticket.is_empty() && self.ticket_time.is_some_and(|time| time.elapsed() < TICKET_MAX_AGE)
    }

    pub fn spawn_ticket(&mut self, purpose: TicketPurpose, account: String, password: String) {
        let tx = self.tx.clone();
        if matches!(purpose, TicketPurpose::Startup) {
            self.sub_title = "logging in...".into();
        }
        tokio::spawn(async move {
            let result = fchat::fetch_ticket(&account, &password).await;
            let _ = tx.send(AppEvent::Ticket(purpose, result));
        });
    }

    /// Connects with the stored ticket, or fetches a new one first.
    pub fn begin_session(&mut self) {
        if self.ticket_is_fresh() && !self.character.is_empty() {
            self.start_connection();
        } else {
            self.spawn_ticket(TicketPurpose::Startup, self.config.account.clone(), self.config.password.clone());
        }
    }

    pub fn start_connection(&mut self) {
        self.generation += 1;
        let generation = self.generation;
        self.conn = None;
        self.online = false;
        self.sub_title = "connecting...".into();
        let identity = Identity {
            account: self.config.account.clone(),
            ticket: self.ticket.clone(),
            character: self.character.clone(),
            client_name: "Terminal Chat Client".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
        };
        let url = self.config.websocket_url();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match Connection::connect(&url, &identity).await {
                Ok((connection, mut events)) => {
                    let _ = tx.send(AppEvent::Connected(generation, Ok(connection)));
                    while let Some(event) = events.recv().await {
                        if tx.send(AppEvent::Server(generation, event)).is_err() {
                            break;
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(AppEvent::Connected(generation, Err(error)));
                }
            }
        });
    }

    pub fn on_ticket(&mut self, purpose: TicketPurpose, result: fchat::Result<LoginResult>) {
        match purpose {
            TicketPurpose::Login { account, password, remember } => match result {
                Err(error) => {
                    if let Some(Screen::Login(screen)) = self.screens.last_mut() {
                        screen.busy = false;
                        screen.message = error.to_string();
                    }
                }
                Ok(login) if login.characters.is_empty() => {
                    if let Some(Screen::Login(screen)) = self.screens.last_mut() {
                        screen.busy = false;
                        screen.message = "This account has no characters".into();
                    }
                }
                Ok(login) => {
                    self.screens.retain(|screen| !matches!(screen, Screen::Login(_)));
                    self.config.account = account;
                    self.config.password = password;
                    self.config.remember_password = remember;
                    let preferred = self.match_character(&self.config.character).unwrap_or(login.default_character.clone());
                    self.accept_login(login);
                    self.choose_character(&preferred);
                }
            },
            TicketPurpose::Startup => match result {
                Err(error) => {
                    self.sub_title = "offline".into();
                    self.show_login(&error.to_string());
                }
                Ok(login) => {
                    let default_character = login.default_character.clone();
                    self.accept_login(login);
                    match self.match_character(&self.config.character) {
                        None => {
                            self.sub_title = "offline".into();
                            let name = self.config.character.clone();
                            self.warn(format!("Character '{name}' not found on this account"));
                            self.choose_character(&default_character);
                        }
                        Some(name) => {
                            self.set_character(&name);
                            self.start_connection();
                        }
                    }
                }
            },
            TicketPurpose::Reconnect => match result {
                Err(error) if !error.is_temporary() => {
                    self.reconnect = None;
                    self.sub_title = "offline".into();
                    self.show_login(&error.to_string());
                }
                Err(error) => self.reconnect_failed(&error.to_string()),
                Ok(login) => {
                    self.accept_login(login);
                    self.start_connection();
                }
            },
        }
    }

    fn accept_login(&mut self, login: LoginResult) {
        self.ticket = login.ticket;
        self.ticket_time = Some(Instant::now());
        self.characters = login.characters;
        self.bookmarks = login.bookmarks;
        self.bookmarks.sort_by_key(|name| name.to_lowercase());
        self.presence.set_bookmarks(&self.bookmarks);
        self.friend_pairs = login.friends;
    }

    pub fn match_character(&self, name: &str) -> Option<String> {
        let name = name.trim().to_lowercase();
        self.characters.iter().find(|character| character.to_lowercase() == name).cloned()
    }

    /// Makes `name` the active character; loads its chats, pins and status.
    pub fn set_character(&mut self, name: &str) {
        if self.config.character != name {
            self.config.character = name.to_string();
            let _ = self.config.save();
        }
        if self.character == name && self.store.base.ends_with(name) {
            return;
        }
        self.character = name.to_string();
        let chats_dir = self.config.data_dir().join("chats");
        self.store = ChatStore::new(name, &chats_dir);
        let base = self.store.base.clone();
        self.pins = Pins::load(base.join("pins.json"));
        self.ads = AdSettings::load(base.join("ads.json"), self.config.show_ads);
        self.ad_drafts = AdDrafts::load(base.join("ad_drafts.json"));
        self.genders = GenderStore::load(chats_dir.join("genders.json"));
        self.own_status = OwnStatus::load(base.join("status.json"));
        self.store.get_or_create(CONSOLE, CONSOLE_KIND, Some("Console"));
        self.reset_view();
    }

    fn reset_view(&mut self) {
        if self.input_mode == InputMode::Ads {
            self.leave_ad_mode();
        }
        self.active_chat = None;
        self.pager.reset();
    }

    pub fn on_character_chosen(&mut self, name: Option<String>) {
        let Some(name) = name else {
            if !self.online && self.conn.is_none() && self.config.character.is_empty() {
                self.show_login("");
            }
            return;
        };
        self.shutdown_client();
        self.config.character = name.clone();
        let _ = self.config.save();
        self.set_character(&name);
        self.begin_session();
    }

    pub fn shutdown_client(&mut self) {
        self.save_files();
        self.online = false;
        self.reconnect = None;
        self.generation += 1;
        self.conn = None;
        self.joining.clear();
        self.pending_open.clear();
        self.pending_create.clear();
        self.joined.clear();
        self.presence = Presence::default();
        self.presence.set_bookmarks(&self.bookmarks);
    }

    pub fn logout(&mut self) {
        self.shutdown_client();
        self.ticket.clear();
        self.config.password.clear();
        let _ = self.config.save();
        self.reset_view();
        self.sub_title = "offline".into();
        self.show_login("");
    }

    pub fn save_files(&mut self) {
        self.genders.save();
        self.store_ad_draft();
        self.ad_drafts.save();
    }

    // ---------------------------------------------------------------- connection events

    pub fn on_connected(&mut self, generation: u64, result: fchat::Result<Connection>) {
        if generation != self.generation {
            return;
        }
        match result {
            Ok(connection) => self.conn = Some(connection),
            Err(error) => self.connection_failed(&error.to_string()),
        }
    }

    fn connection_failed(&mut self, error: &str) {
        if self.reconnect.is_some() {
            self.reconnect_failed(error);
        } else {
            self.sub_title = "offline".into();
            self.notify(error.to_string(), Severity::Error);
        }
    }

    fn reconnect_failed(&mut self, error: &str) {
        let Some(state) = self.reconnect.as_mut() else { return };
        state.attempt += 1;
        let attempt = state.attempt;
        let delay = RECONNECT_DELAYS[attempt.min(RECONNECT_DELAYS.len() - 1)];
        state.due = Instant::now() + Duration::from_secs(delay);
        state.in_flight = false;
        self.log_console(&format!("Reconnect attempt {attempt} failed: {error}"));
    }

    pub fn schedule_reconnect(&mut self, immediate: bool) {
        let delay = if immediate { 0 } else { RECONNECT_DELAYS[0] };
        self.reconnect = Some(Reconnect { attempt: 0, due: Instant::now() + Duration::from_secs(delay), in_flight: false });
    }

    fn connection_lost(&mut self, reason: &str) {
        if self.reconnect.as_ref().is_some_and(|state| state.in_flight) {
            self.conn = None;
            self.reconnect_failed(reason);
            return;
        }
        if !self.online && self.reconnect.is_none() && self.conn.is_none() {
            return;
        }
        self.online = false;
        self.conn = None;
        self.sub_title = "disconnected".into();
        self.log_console("Connection to the chat server lost");
        if self.last_error.is_some_and(|number| NO_RECONNECT_ERRORS.contains(&number)) {
            self.log_console("Not reconnecting automatically after this error, use /reconnect");
            self.notify_titled("", "Connection to the chat server lost, use /reconnect", Severity::Error, 15);
            return;
        }
        self.notify_titled("", "Connection to the chat server lost, reconnecting ...", Severity::Error, 10);
        if self.reconnect.is_none() {
            self.schedule_reconnect(false);
        }
    }

    /// Called every second.
    pub fn on_tick(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|toast| toast.expires > now);
        self.ticks += 1;
        if self.ticks.is_multiple_of(5) && self.images.enabled() && std::env::var_os("TMUX").is_some() {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let output = std::process::Command::new("tmux").args(["display-message", "-p", "#{session_attached}"]).output();
                if let Ok(output) = output {
                    let attached = String::from_utf8_lossy(&output.stdout).trim().parse::<u32>().is_ok_and(|count| count > 0);
                    let _ = tx.send(AppEvent::Attached(attached));
                }
            });
        }

        if let Some(state) = &self.reconnect {
            if !self.online && !state.in_flight {
                if now >= state.due {
                    self.reconnect.as_mut().unwrap().in_flight = true;
                    self.sub_title = "reconnecting...".into();
                    let (account, password) = (self.config.account.clone(), self.config.password.clone());
                    self.spawn_ticket(TicketPurpose::Reconnect, account, password);
                } else {
                    let remaining = (state.due - now).as_secs() + 1;
                    self.sub_title = format!("disconnected \u{2014} reconnecting in {remaining}s (/reconnect: now)");
                }
            }
        }

        if now.duration_since(self.last_gender_save) >= GENDER_SAVE {
            self.last_gender_save = now;
            self.save_files();
        }
        if self.online && self.last_list_refresh.is_none_or(|time| now.duration_since(time) >= LIST_REFRESH) {
            self.last_list_refresh = Some(now);
            let removed = self.store.cleanup(self.config.log_keep_days);
            if removed > 0 {
                self.log_console(&format!("Removed {removed} old log files"));
            }
            self.send(opcode::LIST_OFFICIAL_CHANNELS, None);
            self.send(opcode::LIST_PRIVATE_CHANNELS, None);
        }
        self.pager_op(|pager, src| pager.check_day(src));
    }

    pub fn on_server(&mut self, generation: u64, event: fchat::Event) {
        if generation != self.generation {
            return;
        }
        match event {
            fchat::Event::Closed(reason) => self.connection_lost(&reason),
            fchat::Event::Message(message) => {
                let data = clean_data(message.data);
                self.handle_message(&message.op, data);
            }
        }
    }

    pub fn send(&self, op: &str, data: Option<Value>) {
        if let Some(connection) = &self.conn {
            connection.send(op, data);
        }
    }

    // ---------------------------------------------------------------- server messages

    fn handle_message(&mut self, op: &str, mut data: Value) {
        if let Some(channel) = data.get("channel").and_then(Value::as_str).map(String::from) {
            data["channel"] = Value::String(self.canonical_channel(&channel));
        }
        match op {
            opcode::PING => {
                if let Some(connection) = &self.conn {
                    connection.send_priority(opcode::PING, None);
                }
            }
            opcode::IDENTIFY => self.on_identified(),
            opcode::PRIVATE_MESSAGE => {
                self.on_private_message(text_of(&data, "character"), text_of(&data, "message"));
            }
            opcode::CHANNEL_MESSAGE => {
                self.on_channel_message(text_of(&data, "channel"), text_of(&data, "character"), text_of(&data, "message"));
            }
            opcode::ROLEPLAY_AD => {
                self.on_ad(text_of(&data, "channel"), text_of(&data, "character"), text_of(&data, "message"));
            }
            opcode::ROLL => self.on_roll(&data),
            opcode::LIST_OFFICIAL_CHANNELS => {
                for channel in data.get("channels").and_then(Value::as_array).into_iter().flatten() {
                    let name = text_of(channel, "name");
                    self.official.entry(name.to_string()).or_insert_with(|| name.to_string());
                }
            }
            opcode::LIST_PRIVATE_CHANNELS => {
                for channel in data.get("channels").and_then(Value::as_array).into_iter().flatten() {
                    let code = text_of(channel, "name");
                    self.open_private.entry(code.to_string()).or_insert_with(|| text_of(channel, "title").to_string());
                }
            }
            opcode::JOIN_CHANNEL => self.on_join(&data),
            opcode::LEAVE_CHANNEL => {
                let (code, character) = (text_of(&data, "channel"), text_of(&data, "character"));
                if self.is_me(character) {
                    self.on_left(code);
                } else {
                    self.presence.remove_member(code, character);
                }
            }
            opcode::LIST_CHARACTERS => {
                for entry in data.get("characters").and_then(Value::as_array).into_iter().flatten() {
                    let field = |index: usize| entry.get(index).and_then(Value::as_str);
                    let Some(name) = field(0) else { continue };
                    self.presence.set_online(name, field(2).unwrap_or("online"), field(3).unwrap_or(""));
                    self.genders.set(name, field(1));
                }
            }
            opcode::USER_CONNECTED => {
                let name = text_of(&data, "identity").to_string();
                self.presence.set_online(&name, data.get("status").and_then(Value::as_str).unwrap_or("online"), "");
                self.genders.set(&name, data.get("gender").and_then(Value::as_str));
                let friend = self.is_friend(&name);
                self.on_presence_notice(&name, "is online", friend);
            }
            opcode::GONE_OFFLINE => {
                let name = text_of(&data, "character").to_string();
                let friend = self.is_friend(&name);
                self.presence.set_offline(&name);
                self.on_presence_notice(&name, "went offline", friend);
            }
            opcode::FRIENDS_LIST => self.on_friends(&data),
            opcode::STATUS => {
                self.presence.set_status(text_of(&data, "character"), text_of(&data, "status"), text_of(&data, "statusmsg"));
            }
            opcode::CHANNEL_DESCRIPTION => {
                self.descriptions.insert(text_of(&data, "channel").to_string(), text_of(&data, "description").to_string());
            }
            opcode::INITIAL_CHANNEL_DATA => {
                let names: Vec<String> = data
                    .get("users")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|user| text_of(user, "identity").to_string())
                    .collect();
                self.presence.set_members(text_of(&data, "channel"), &names);
            }
            opcode::LIST_OPS => {
                let names: Vec<String> = data
                    .get("oplist")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|name| name.as_str().map(String::from))
                    .collect();
                self.presence.set_ops(text_of(&data, "channel"), &names);
            }
            opcode::PROMOTE_OP => {
                let (code, name) = (text_of(&data, "channel").to_string(), text_of(&data, "character").to_string());
                self.presence.add_op(&code, &name);
                self.on_channel_event(&code, &format!("{name} is now a channel operator"), "");
            }
            opcode::DEMOTE_OP => {
                let (code, name) = (text_of(&data, "channel").to_string(), text_of(&data, "character").to_string());
                self.presence.remove_op(&code, &name);
                self.on_channel_event(&code, &format!("{name} is no longer a channel operator"), "");
            }
            opcode::INVITE => {
                let code = text_of(&data, "name").to_string();
                let title = match text_of(&data, "title") {
                    "" => code.clone(),
                    title => title.to_string(),
                };
                self.on_invited(text_of(&data, "sender"), &code, &title);
            }
            opcode::KICK | opcode::BAN => {
                let verb = if op == opcode::KICK { "kicked" } else { "banned" };
                let name = text_of(&data, "character").to_string();
                let operator = data.get("operator").and_then(Value::as_str).unwrap_or("?");
                let text = format!("{operator} {verb} {name}");
                self.on_channel_event(text_of(&data, "channel"), &text, &name);
            }
            opcode::TIMEOUT => {
                let name = text_of(&data, "character").to_string();
                let operator = data.get("operator").and_then(Value::as_str).unwrap_or("?");
                let length = data.get("length").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                let text = format!("{operator} timed out {name} for {length} minutes");
                self.on_channel_event(text_of(&data, "channel"), &text, &name);
            }
            opcode::BANLIST => {
                let list = text_of(&data, "banlist").to_string();
                let code = text_of(&data, "channel").to_string();
                let text = if list.is_empty() { "The ban list is empty".to_string() } else { format!("Ban list: {list}") };
                if self.store.find(CHANNEL, &code).is_some() {
                    self.on_channel_message(&code, "System", &text);
                } else {
                    self.log_console(&text);
                }
            }
            opcode::ERROR => self.on_error(&data),
            opcode::SERVER_HELLO => {
                let message = data.get("message").and_then(Value::as_str).unwrap_or("Server hello").to_string();
                self.log_console(&message);
            }
            opcode::NUMBER_OF_CONNECTED_USERS => {
                let count = data.get("count").map(|v| v.to_string()).unwrap_or_else(|| "?".into());
                self.log_console(&format!("{count} users online"));
            }
            opcode::BROADCAST => {
                let message = text_of(&data, "message").to_string();
                self.notify_titled("Broadcast", message.clone(), Severity::Information, 20);
                self.log_console(&format!("Broadcast: {message}"));
            }
            opcode::SITE_EVENT => self.on_site_event(&data),
            opcode::VARIABLES => {
                if let Some(name) = data.get("variable").and_then(Value::as_str) {
                    self.variables.insert(name.to_string(), data.get("value").cloned().unwrap_or(Value::Null));
                }
            }
            opcode::SYSTEM_MESSAGE => {
                let message = text_of(&data, "message").to_string();
                let channel = text_of(&data, "channel");
                if !channel.is_empty() && self.store.find(CHANNEL, channel).is_some() {
                    self.on_channel_message(channel, "System", &message);
                } else {
                    self.notify_titled("System", message.clone(), Severity::Information, TOAST_SECONDS);
                    self.log_console(&message);
                }
            }
            _ => {}
        }
    }

    fn on_identified(&mut self) {
        self.online = true;
        self.last_list_refresh = None;
        self.update_subtitle();
        if !self.own_status.is_default() {
            self.send_status();
        }
        let was_reconnect = self.reconnect.take().is_some();
        self.send(opcode::LIST_OFFICIAL_CHANNELS, None);
        self.send(opcode::LIST_PRIVATE_CHANNELS, None);
        if was_reconnect {
            let channels: Vec<String> =
                self.store.chats.iter().filter(|chat| chat.kind == CHANNEL).map(|chat| chat.key.clone()).collect();
            for code in channels {
                self.join(&code, false);
            }
            let character = self.character.clone();
            self.log_console(&format!("Reconnected as [user]{character}[/user]"));
            self.notify("Reconnected to the chat server", Severity::Information);
            return;
        }
        self.restore_pins();
        let defaults = self.config.default_channels.clone();
        for (index, channel) in defaults.iter().enumerate() {
            self.join(channel, index == 0);
        }
        for note in std::mem::take(&mut self.startup_notes) {
            self.log_console(&note);
        }
        let character = self.character.clone();
        self.log_console(&format!("Connected as [user]{character}[/user]"));
        if defaults.is_empty() && self.active_chat.is_none() {
            if let Some(first) = self.first_chat_id() {
                self.open_chat(&first);
            }
        }
    }

    pub fn update_subtitle(&mut self) {
        self.sub_title = format!("{} \u{2014} as {}", self.own_status.status, self.character);
    }

    fn on_error(&mut self, data: &Value) {
        if let Some(number) = data.get("number").and_then(Value::as_i64) {
            self.last_error = Some(number);
        }
        let number = data.get("number").map(|v| v.to_string()).unwrap_or_else(|| "null".into());
        let text = format!("Error {number}: {}", text_of(data, "message"));
        self.notify(text.clone(), Severity::Error);
        self.log_console(&text);
    }

    fn on_site_event(&mut self, data: &Value) {
        let kind = data.get("type").and_then(Value::as_str).unwrap_or("event");
        let name = data.get("name").and_then(Value::as_str).filter(|n| !n.is_empty());
        if let (true, Some(name)) = (kind == "bookmarkadd" || kind == "bookmarkremove", name) {
            if kind == "bookmarkadd" {
                self.presence.add_bookmark(name);
                if !self.bookmarks.iter().any(|b| b.eq_ignore_ascii_case(name)) {
                    self.bookmarks.push(name.to_string());
                }
            } else {
                self.presence.remove_bookmark(name);
                self.bookmarks.retain(|b| !b.eq_ignore_ascii_case(name));
            }
        }
        let who = ["name", "sender", "character"].iter().map(|key| text_of(data, key)).find(|v| !v.is_empty()).unwrap_or("");
        let details = ["subject", "title"].iter().map(|key| text_of(data, key)).find(|v| !v.is_empty()).unwrap_or("");
        let mut text = format!("Site event: {kind}");
        if !who.is_empty() {
            text.push_str(&format!(" from [user]{who}[/user]"));
        }
        if !details.is_empty() {
            text.push_str(&format!(" - {details}"));
        }
        self.log_console(&text);
    }

    fn account_friends(&self) -> Option<Vec<String>> {
        let pairs = self.friend_pairs.as_ref().filter(|pairs| !pairs.is_empty())?;
        let own: HashSet<String> = self.characters.iter().map(|name| name.to_lowercase()).collect();
        let mut friends: Vec<String> = Vec::new();
        for (source, dest) in pairs {
            let chosen = if own.contains(&source.to_lowercase()) {
                dest
            } else if own.contains(&dest.to_lowercase()) || own.is_empty() {
                source
            } else {
                continue;
            };
            if !friends.contains(chosen) {
                friends.push(chosen.clone());
            }
        }
        friends.sort_by_key(|name| name.to_lowercase());
        (!friends.is_empty()).then_some(friends)
    }

    fn on_friends(&mut self, data: &Value) {
        let friends = self.account_friends().unwrap_or_else(|| {
            data.get("characters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|name| !self.presence.bookmarks.contains(&name.to_lowercase()))
                .map(String::from)
                .collect()
        });
        self.presence.set_friends(&friends);
    }

    fn is_friend(&self, name: &str) -> bool {
        self.presence.friends.contains(&name.to_lowercase()) && !self.is_me(name)
    }

    pub fn is_me(&self, name: &str) -> bool {
        !name.is_empty() && name.eq_ignore_ascii_case(&self.character)
    }

    fn on_roll(&mut self, data: &Value) {
        let message = text_of(data, "message").to_string();
        if let Some(channel) = data.get("channel").and_then(Value::as_str) {
            self.on_channel_message(channel, "System", &message);
            return;
        }
        let character = text_of(data, "character");
        let partner = if self.is_me(character) { text_of(data, "recipient") } else { character };
        if !partner.is_empty() {
            let partner = partner.to_string();
            self.on_private_system(&partner, &message);
        }
    }

    fn on_join(&mut self, data: &Value) {
        let code = text_of(data, "channel").to_string();
        let identity = data.get("character").and_then(|c| c.get("identity")).and_then(Value::as_str).unwrap_or("");
        if !self.is_me(identity) {
            self.presence.add_member(&code, identity);
            return;
        }
        let title = match text_of(data, "title") {
            "" => code.clone(),
            title => title.to_string(),
        };
        self.on_joined(&code, &title);
    }

    // ---------------------------------------------------------------- channels and chats

    pub fn canonical_channel(&self, code: &str) -> String {
        let lowered = code.to_lowercase();
        let known = self
            .joined
            .keys()
            .chain(self.joining.iter())
            .chain(self.pending_open.iter())
            .chain(self.store.chats.iter().filter(|chat| chat.kind == CHANNEL).map(|chat| &chat.key));
        known.into_iter().find(|key| key.to_lowercase() == lowered).cloned().unwrap_or_else(|| code.to_string())
    }

    pub fn resolve_channel(&self, name_or_code: &str) -> String {
        let lowered = name_or_code.to_lowercase();
        let by_name = self
            .joined
            .iter()
            .chain(self.open_private.iter())
            .find(|(_, title)| title.to_lowercase() == lowered)
            .map(|(code, _)| code.clone())
            .or_else(|| self.official.contains_key(name_or_code).then(|| name_or_code.to_string()));
        if let Some(code) = by_name {
            return code;
        }
        self.joined
            .keys()
            .find(|code| code.to_lowercase() == lowered)
            .cloned()
            .or_else(|| self.open_private.contains_key(name_or_code).then(|| name_or_code.to_string()))
            .or_else(|| self.official.contains_key(name_or_code).then(|| name_or_code.to_string()))
            .unwrap_or_else(|| name_or_code.to_string())
    }

    pub fn channel_title(&self, code: &str) -> Option<String> {
        self.joined.get(code).cloned().or_else(|| {
            let lowered = code.to_lowercase();
            self.joined.iter().find(|(key, _)| key.to_lowercase() == lowered).map(|(_, title)| title.clone())
        })
    }

    pub fn join(&mut self, name_or_code: &str, open_chat: bool) {
        let code = self.resolve_channel(name_or_code);
        if self.joined.contains_key(&code) {
            if open_chat {
                self.open_chat(&make_id(CHANNEL, &code));
            }
            return;
        }
        if open_chat {
            self.pending_open.insert(code.clone());
        }
        if self.joining.contains(&code) {
            return;
        }
        self.joining.insert(code.clone());
        self.send(opcode::JOIN_CHANNEL, Some(json!({"channel": code})));
    }

    pub fn leave(&mut self, code: &str) {
        self.joining.remove(code);
        self.joined.remove(code);
        self.presence.drop_channel(code);
        self.send(opcode::LEAVE_CHANNEL, Some(json!({"channel": code})));
    }

    fn restore_pins(&mut self) {
        let pins = self.pins.items.clone();
        for pin in pins {
            if pin.kind == CHANNEL {
                self.store.get_or_create(&pin.key, CHANNEL, pin.title.as_deref());
                self.join(&pin.key, false);
            } else {
                self.store.get_or_create(&pin.key, USER, pin.title.as_deref());
            }
        }
    }

    fn on_joined(&mut self, code: &str, title: &str) {
        self.joining.remove(code);
        self.joined.insert(code.to_string(), title.to_string());
        let chat_id = self.store.get_or_create(code, CHANNEL, Some(title)).id();
        if self.pending_open.remove(code) | self.pending_create.remove(&title.to_lowercase()) {
            self.open_chat(&chat_id);
        }
    }

    fn on_left(&mut self, code: &str) {
        self.joined.remove(code);
        self.presence.drop_channel(code);
        let chat_id = make_id(CHANNEL, code);
        self.store.remove(&chat_id);
        self.chat_removed(&chat_id);
    }

    pub fn create_room(&mut self, title: &str) {
        self.pending_create.insert(title.to_lowercase());
        self.send(opcode::CREATE_PRIVATE_CHANNEL, Some(json!({"channel": title})));
    }

    fn on_channel_event(&mut self, code: &str, text: &str, removed: &str) {
        if !removed.is_empty() {
            self.presence.remove_member(code, removed);
            if self.is_me(removed) {
                self.notify_titled("", text, Severity::Warning, 15);
                self.on_left(code);
                return;
            }
        }
        if self.store.find(CHANNEL, code).is_some() {
            self.on_channel_message(code, "System", text);
        }
    }

    pub fn invite(&mut self, code: &str, name: &str) {
        self.send(opcode::INVITE, Some(json!({"channel": code, "character": name})));
    }

    fn on_invited(&mut self, sender: &str, code: &str, title: &str) {
        self.open_private.entry(code.to_string()).or_insert_with(|| title.to_string());
        let title = html_unescape(title);
        self.notify_titled("Invitation", format!("{sender} invited you to {title}\n/join {title}"), Severity::Information, 30);
        self.log_console(&format!("[user]{sender}[/user] invited you to {title} - /join {title}"));
    }

    pub fn log_console(&mut self, text: &str) {
        let Some(chat) = self.store.find(CONSOLE, CONSOLE).cloned() else { return };
        let line = self.store.append(&chat, "System", text);
        self.show_line(&chat.id(), &line, true);
    }

    pub fn find_online(&self, name: &str) -> Option<String> {
        self.presence.find_online(name)
    }

    pub fn open_private(&mut self, character: &str) {
        let name = match self.find_online(character) {
            Some(name) => name,
            None => match self.store.find_user(character) {
                Some(existing) => existing.key.clone(),
                None => {
                    self.warn(format!("'{character}' is not online or does not exist"));
                    return;
                }
            },
        };
        let chat_id = self.store.get_or_create(&name, USER, None).id();
        self.open_chat(&chat_id);
    }

    pub fn check_online(&mut self) -> bool {
        if !self.online {
            self.warn("Not connected, nothing was sent");
        }
        self.online
    }

    /// Sends a chat or private message and logs it. Returns false if nothing was sent.
    pub fn send_message(&mut self, chat_id: &str, message: &str) -> bool {
        if !self.check_online() {
            return false;
        }
        let Some(chat) = self.store.get(chat_id).cloned() else { return false };
        if chat.kind == USER && self.find_online(&chat.key).is_none() {
            self.warn(format!("{} is offline, message not sent", chat.title));
            return false;
        }
        if chat.kind == CHANNEL {
            self.send(opcode::CHANNEL_MESSAGE, Some(json!({"channel": chat.key, "message": message})));
        } else {
            self.send(opcode::PRIVATE_MESSAGE, Some(json!({"recipient": chat.key, "message": message})));
        }
        let line = self.store.append(&chat, &self.character, message);
        self.show_line(chat_id, &line, true);
        true
    }

    pub fn ad_wait_seconds(&self, code: &str) -> u64 {
        let flood = self.variables.get("lfrp_flood").and_then(Value::as_f64).unwrap_or(600.0);
        match self.last_ad.get(&code.to_lowercase()) {
            Some(time) => (flood - time.elapsed().as_secs_f64() + 0.999).max(0.0) as u64,
            None => 0,
        }
    }

    pub fn send_ad(&mut self, chat_id: &str, message: &str) {
        if !self.check_online() {
            return;
        }
        let Some(chat) = self.store.get(chat_id).cloned() else { return };
        self.send(opcode::ROLEPLAY_AD, Some(json!({"channel": chat.key, "message": message})));
        self.last_ad.insert(chat.key.to_lowercase(), Instant::now());
        let line = self.store.append(&chat, &format!("{AD_PREFIX}{}", self.character), message);
        self.show_line(chat_id, &line, true);
    }

    pub fn roll(&mut self, chat_id: &str, dice: &str) {
        let Some(chat) = self.store.get(chat_id).cloned() else { return };
        let payload = if chat.kind == CHANNEL {
            json!({"channel": chat.key, "dice": dice})
        } else {
            json!({"recipient": chat.key, "dice": dice})
        };
        self.send(opcode::ROLL, Some(payload));
    }

    fn on_private_system(&mut self, partner: &str, message: &str) {
        let chat = self.store.get_or_create(partner, USER, None).clone();
        let line = self.store.append(&chat, "System", message);
        self.show_line(&chat.id(), &line, true);
    }

    fn on_presence_notice(&mut self, name: &str, text: &str, friend: bool) {
        let chat = self.store.find_user(name).cloned();
        if let Some(chat) = &chat {
            let line = self.store.append(chat, "System", &format!("[user]{name}[/user] {text}"));
            self.show_line(&chat.id(), &line, false);
        }
        if chat.is_some() || friend {
            self.log_console(&format!("[user]{name}[/user] {text}"));
        }
    }

    fn on_private_message(&mut self, sender: &str, message: &str) {
        let chat = self.store.get_or_create(sender, USER, None).clone();
        let line = self.store.append(&chat, sender, message);
        self.show_line(&chat.id(), &line, true);
    }

    fn channel_chat(&mut self, code: &str) -> crate::store::Chat {
        if let Some(chat) = self.store.find(CHANNEL, code) {
            return chat.clone();
        }
        let title = self.channel_title(code);
        self.store.get_or_create(code, CHANNEL, title.as_deref()).clone()
    }

    fn on_channel_message(&mut self, code: &str, sender: &str, message: &str) {
        let chat = self.channel_chat(code);
        let line = self.store.append(&chat, sender, message);
        self.show_line(&chat.id(), &line, true);
    }

    fn on_ad(&mut self, code: &str, sender: &str, message: &str) {
        let chat = self.channel_chat(code);
        let line = self.store.append(&chat, &format!("{AD_PREFIX}{sender}"), message);
        if self.ads.visible(code) {
            self.show_line(&chat.id(), &line, true);
        }
    }

    pub fn send_status(&mut self) {
        let payload = json!({"status": self.own_status.status, "statusmsg": self.own_status.message});
        self.send(opcode::STATUS, Some(payload));
    }

    // ---------------------------------------------------------------- chat view

    /// Runs `f` with the pager and a source for the open chat.
    pub fn pager_op<R>(&mut self, f: impl FnOnce(&mut Pager, &Source) -> R) -> Option<R> {
        let id = self.active_chat.clone()?;
        let chat = self.store.get(&id)?.clone();
        let App { pager, store, ads, genders, .. } = self;
        let name_style = |name: &str| gender_style(genders.get(name));
        let source = Source { store, ads, chat: &chat, name_style: &name_style };
        Some(f(pager, &source))
    }

    pub fn name_style(&self, name: &str) -> ratatui::style::Style {
        gender_style(self.genders.get(name))
    }

    pub fn first_chat_id(&self) -> Option<String> {
        [CHANNEL, USER, CONSOLE]
            .iter()
            .find_map(|kind| self.store.chats.iter().find(|chat| chat.kind == *kind))
            .map(|chat| chat.id())
    }

    /// Chat ids in the order of the chat list: console, channels, private chats.
    pub fn list_ids(&self) -> Vec<Option<String>> {
        let mut ids = Vec::new();
        for kind in [CONSOLE, CHANNEL, USER] {
            let group: Vec<String> = self.store.chats.iter().filter(|chat| chat.kind == kind).map(|chat| chat.id()).collect();
            if group.is_empty() {
                continue;
            }
            if !ids.is_empty() {
                ids.push(None);
            }
            ids.extend(group.into_iter().map(Some));
        }
        ids
    }

    pub fn current_chat(&self) -> Option<&crate::store::Chat> {
        self.active_chat.as_ref().and_then(|id| self.store.get(id))
    }

    pub fn open_chat(&mut self, chat_id: &str) {
        let Some(chat) = self.store.get(chat_id).cloned() else { return };
        if self.input_mode == InputMode::Ads && self.ad_channel.as_deref() != Some(chat.key.as_str()) {
            if chat.kind == CHANNEL {
                self.store_ad_draft();
                self.ad_channel = Some(chat.key.clone());
                let draft = self.ad_drafts.get(&chat.key);
                self.input.set_text(&draft);
            } else {
                self.leave_ad_mode();
            }
        }
        self.active_chat = Some(chat_id.to_string());
        self.input.use_history(chat_id);
        if let Some(chat) = self.store.get_mut(chat_id) {
            chat.unread = false;
        }
        self.sidebar_tab = if chat.kind == CHANNEL { SidebarTab::Channel } else { SidebarTab::Friends };
        let id = chat_id.to_string();
        self.pager_op(|pager, src| pager.open(src, &id, None, true));
        self.update_input_mode();
    }

    pub fn show_line(&mut self, chat_id: &str, line: &str, mark_unread: bool) {
        if self.active_chat.as_deref() == Some(chat_id) {
            let line = line.to_string();
            self.pager_op(|pager, src| pager.append(src, &line));
            return;
        }
        if mark_unread {
            if let Some(chat) = self.store.get_mut(chat_id) {
                chat.unread = true;
            }
        }
    }

    fn chat_removed(&mut self, chat_id: &str) {
        if self.active_chat.as_deref() != Some(chat_id) {
            return;
        }
        self.active_chat = None;
        match self.first_chat_id() {
            Some(first) => self.open_chat(&first),
            None => self.pager.reset(),
        }
    }

    pub fn close_chat(&mut self) {
        let Some(chat) = self.current_chat().cloned() else { return };
        if chat.kind == CONSOLE {
            self.warn("The console cannot be closed");
            return;
        }
        if chat.kind == CHANNEL {
            self.leave(&chat.key);
        }
        self.store.remove(&chat.id());
        self.chat_removed(&chat.id());
    }

    pub fn select_relative_chat(&mut self, step: isize) {
        let ids: Vec<String> = self.list_ids().into_iter().flatten().collect();
        if ids.is_empty() {
            return;
        }
        let current = self.active_chat.as_ref().and_then(|id| ids.iter().position(|other| other == id)).unwrap_or(0);
        let next = (current as isize + step).rem_euclid(ids.len() as isize) as usize;
        let id = ids[next].clone();
        self.open_chat(&id);
    }

    // ---------------------------------------------------------------- links

    pub fn open_link(&mut self, url: &str, key: Option<EntryKey>) {
        if let Some(name) = url.strip_prefix(USER_LINK) {
            self.open_private(name);
            return;
        }
        if let Some(code) = url.strip_prefix(CHANNEL_LINK) {
            self.join(code, true);
            return;
        }
        if url == SPOILER_LINK {
            if let Some(key) = key {
                self.pager.reveal_spoilers(key);
            }
            return;
        }
        if !is_safe_url(url) {
            self.notify_titled("", format!("Not copying link, only http and https are allowed: {url}"), Severity::Warning, 10);
            return;
        }
        self.copy_to_clipboard(url);
        self.notify_titled("Link copied to clipboard", url, Severity::Information, 15);
    }

    pub fn show_profile(&mut self, name: &str) {
        if self.screens.iter().any(|screen| matches!(screen, Screen::Profile(_))) {
            return;
        }
        self.screens.push(Screen::Profile(crate::screens::ProfileScreen::new(name)));
        self.spawn_profile(name);
    }

    fn spawn_profile(&mut self, name: &str) {
        let tx = self.tx.clone();
        let name = name.to_string();
        let account = self.config.account.clone();
        let password = self.config.password.clone();
        let cached = self.ticket_is_fresh().then(|| self.ticket.clone());
        let mapping_file = paths::cache_dir().join("mapping-list.json");
        tokio::spawn(async move {
            let result = async {
                let mut ticket = match cached {
                    Some(ticket) => ticket,
                    None => fchat::fetch_ticket(&account, &password).await.map_err(|e| e.to_string())?.ticket,
                };
                let first = fchat::fetch_profile(&account, &ticket, &name, &mapping_file).await;
                match first {
                    Err(error) if error.to_string().to_lowercase().contains("ticket") => {
                        ticket = fchat::fetch_ticket(&account, &password).await.map_err(|e| e.to_string())?.ticket;
                        fchat::fetch_profile(&account, &ticket, &name, &mapping_file).await.map_err(|e| e.to_string())
                    }
                    other => other.map_err(|e| e.to_string()),
                }
            }
            .await;
            let _ = tx.send(AppEvent::Profile(name, result));
        });
    }

    pub fn on_profile(&mut self, name: &str, result: Result<fchat::Profile, String>) {
        for screen in &mut self.screens {
            if let Screen::Profile(profile) = screen {
                if profile.character == name {
                    profile.result = Some(result);
                    return;
                }
            }
        }
    }

    pub fn show_info(&mut self, title: &str, lines: Vec<Vec<Seg>>, wide: bool) {
        self.screens.push(Screen::Info(InfoScreen::new(title, lines, wide)));
    }

    // ---------------------------------------------------------------- input modes

    pub fn store_ad_draft(&mut self) {
        if self.input_mode != InputMode::Ads {
            return;
        }
        let Some(code) = self.ad_channel.clone() else { return };
        let text = self.input.text().to_string();
        if !text.starts_with('/') {
            self.ad_drafts.set(&code, &text);
        }
    }

    pub fn leave_ad_mode(&mut self) {
        self.store_ad_draft();
        self.ad_drafts.save();
        self.input_mode = InputMode::Chat;
        self.ad_channel = None;
        let draft = self.chat_draft.clone();
        self.input.set_text(&draft);
        self.update_input_mode();
    }

    pub fn set_input_mode(&mut self, mode: InputMode) {
        if mode == self.input_mode {
            return;
        }
        if mode == InputMode::Chat {
            self.leave_ad_mode();
            return;
        }
        let Some(chat) = self.current_chat().cloned().filter(|chat| chat.kind == CHANNEL) else {
            self.warn("Ads can only be posted in channels");
            return;
        };
        self.chat_draft = self.input.text().to_string();
        self.input_mode = InputMode::Ads;
        self.ad_channel = Some(chat.key.clone());
        let draft = self.ad_drafts.get(&chat.key);
        self.input.set_text(&draft);
        self.update_input_mode();
    }

    pub fn toggle_ad_mode(&mut self) {
        let mode = if self.input_mode == InputMode::Ads { InputMode::Chat } else { InputMode::Ads };
        self.set_input_mode(mode);
    }

    pub fn update_input_mode(&mut self) {
        self.input.history_enabled = self.input_mode == InputMode::Chat;
    }

    /// Server limit of the input, if any.
    pub fn input_limit(&self) -> Option<usize> {
        let chat = self.current_chat()?;
        if chat.kind == CONSOLE {
            return None;
        }
        let name = if self.input_mode == InputMode::Ads {
            "lfrp_max"
        } else if chat.kind == CHANNEL {
            "chat_max"
        } else {
            "priv_max"
        };
        Some(self.variables.get(name).and_then(Value::as_u64).unwrap_or(50000) as usize)
    }

    /// Submits the input (enter).
    pub fn submit(&mut self) {
        let value = self.input.text().to_string();
        if !value.trim().is_empty() && !value.starts_with('/') && !self.online {
            self.warn("Not connected, message not sent (/reconnect to try now)");
            return;
        }
        if self.input_mode == InputMode::Ads && !value.starts_with('/') {
            if !value.trim().is_empty() {
                self.cmd_ad(value.trim());
            }
            self.store_ad_draft();
            self.ad_drafts.save();
            return;
        }

        let text = value.trim().to_string();
        self.input.clear();
        if text.is_empty() {
            return;
        }
        self.input.remember(&text);

        if text.starts_with('/') {
            self.run_command(&text);
            if self.input_mode == InputMode::Ads && self.input.text().is_empty() {
                if let Some(code) = self.ad_channel.clone() {
                    let draft = self.ad_drafts.get(&code);
                    self.input.set_text(&draft);
                }
            }
            return;
        }

        let Some(chat) = self.current_chat().cloned() else {
            self.warn("No chat open");
            return;
        };
        if chat.kind == CONSOLE {
            self.warn("The console is read-only, use /commands here");
            return;
        }
        if !self.send_message(&chat.id(), &text) {
            self.input.set_text(&text);
        }
    }
}

pub const CONSOLE_KIND: &str = CONSOLE;

fn default_variables() -> HashMap<String, Value> {
    [("lfrp_flood", json!(600)), ("lfrp_max", json!(50000)), ("chat_max", json!(4096)), ("priv_max", json!(50000))]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}
