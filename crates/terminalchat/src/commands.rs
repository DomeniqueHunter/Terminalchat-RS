//! Slash commands.

use fchat::opcode;
use serde_json::{json, Value};

use crate::app::{App, Severity};
use crate::bbcode::{to_segs, Seg};
use crate::sanitize::html_unescape;
use crate::screens::{bbcode_items, emoji_items, text_lines, EditScreen, PickItem, PickerKind, PickerScreen, Screen};
use crate::store::{CHANNEL, CONSOLE, USER};
use crate::stores::{MAX_STATUS_MESSAGE, VALID_STATUSES};
use crate::style::{bold, dim, status_style};

pub const COMMAND_INFO: &[(&str, &str, &str)] = &[
    ("join", "<channel>", "join a channel (name or code)"),
    ("leave", "", "leave / close the current chat"),
    ("priv", "<name>", "open a private chat"),
    ("invite", "<name>", "invite a character into the current channel"),
    ("code", "", "copy the current channel as [session] link"),
    ("makeroom", "<title>", "create a private channel"),
    ("openroom", "", "make the current private channel public"),
    ("closeroom", "", "make the current private channel invite-only"),
    ("kick", "<name>", "kick a character from the current channel"),
    ("ban", "<name>", "ban a character from the current channel"),
    ("unban", "<name>", "unban a character"),
    ("banlist", "", "show the ban list of the current channel"),
    ("timeout", "<name> <minutes>", "time out a character"),
    ("op", "<name>", "make a character channel operator"),
    ("deop", "<name>", "remove channel operator rights"),
    ("setowner", "<name>", "hand the channel over to a character"),
    ("setdescription", "<text>", "set the channel description"),
    ("setmode", "<chat|ads|both>", "allow chat, ads or both"),
    ("me", "<text>", "emote in the current chat"),
    ("ad", "<text>", "post a role play ad in the current channel"),
    ("roll", "[dice]", "roll dice on the server, e.g. 2d6+3 (default 1d6)"),
    ("bottle", "", "spin the bottle (channels only)"),
    ("status", "[status] [text]", "set status and/or status message"),
    ("status:flag", "<status>", "change only the status"),
    ("status:message", "[text]", "change only the message; without text opens an editor"),
    ("channels", "", "open the channel list"),
    ("users", "", "show/hide friends and channel users"),
    ("pin", "", "pin the current chat, restored on start"),
    ("unpin", "", "unpin the current chat"),
    ("pins", "", "show pinned chats"),
    ("ads", "[on|off]", "show or hide ads in the current channel"),
    ("char", "[name]", "switch character"),
    ("reconnect", "", "reconnect to the chat server now"),
    ("logout", "", "log out and forget password"),
    ("minimize", "", "detach, keep running in background"),
    ("bbcode", "[search]", "list BBCode tags and insert one into the input (ctrl+b)"),
    ("emoji", "[search]", "pick an emoji and insert it into the input (ctrl+e)"),
    ("profile", "[name]", "show the profile of the chat partner or of a character"),
    ("chatstatus", "", "show the partner's status or the channel description"),
    ("help", "", "show all commands"),
    ("shortcuts", "", "show all key bindings"),
    ("quit", "", "exit"),
];

pub const NAME_COMMANDS: &[&str] =
    &["priv", "profile", "invite", "kick", "ban", "unban", "timeout", "op", "deop", "setowner"];

/// The footer bar: (ctrl+key, label).
pub const FOOTER_KEYS: &[(char, &str)] = &[
    ('o', "Channels"),
    ('l', "Close chat"),
    ('f', "Users"),
    ('t', "Minimize"),
    ('a', "Chat/Ads"),
    ('e', "Emoji"),
    ('h', "Help"),
    ('p', "Profile"),
];

pub const SHORTCUT_INFO: &[(&str, &str)] = &[
    ("enter", "send the message (Ads mode: post the ad)"),
    ("ctrl+j / alt+enter", "new line"),
    ("up / down", "move between lines, then browse sent input"),
    ("tab / shift+tab", "complete a command or character name (next / previous)"),
    ("ctrl+c", "clear the input (ctrl+z brings it back)"),
    ("ctrl+a", "switch input between Chat and Ads"),
    ("ctrl+e", "emoji picker, inserts at the cursor"),
    ("ctrl+h / f1", "command list"),
    ("ctrl+b", "BBCode picker"),
    ("alt+b / i / u / s", "insert bold / italic / underline / strike"),
    ("alt+p / alt+q", "insert spoiler / quote"),
    ("click name / #channel", "open private chat / join channel"),
    ("click spoiler", "reveal it"),
    ("ctrl+o", "open the channel list"),
    ("ctrl+l", "leave / close the current chat"),
    ("ctrl+f", "show/hide friends and channel users"),
    ("ctrl+t", "minimize (detach, keep running)"),
    ("ctrl+p", "profile of the private chat partner"),
    ("page up / page down", "scroll the chat"),
    ("ctrl+page up / down", "jump to the start of the log / to now"),
    ("alt+up / alt+down", "previous / next chat"),
    ("escape", "close a dialog"),
    ("click title bar", "show/hide your status message"),
    ("click link", "copy the link to the clipboard"),
    ("shift + mouse", "select text in the terminal"),
    ("ctrl+q", "quit"),
];

pub fn command_usage(name: &str, args: &str) -> String {
    format!("/{name} {args}").trim_end().to_string()
}

pub fn resolve_alias(name: &str) -> String {
    let name = name.to_lowercase();
    match name.as_str() {
        "profil" => "profile",
        "close" => "leave",
        "create" => "makeroom",
        "detach" => "minimize",
        "keys" => "shortcuts",
        _ => name.as_str(),
    }
    .to_string()
}

pub fn matching_commands(prefix: &str) -> Vec<&'static (&'static str, &'static str, &'static str)> {
    let prefix = prefix.to_lowercase();
    COMMAND_INFO.iter().filter(|entry| entry.0.starts_with(&prefix)).collect()
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    let bytes: Vec<char> = text.chars().collect();
    if bytes.len() >= 2 && bytes[0] == bytes[bytes.len() - 1] && (bytes[0] == '"' || bytes[0] == '\'') {
        return bytes[1..bytes.len() - 1].iter().collect::<String>().trim().to_string();
    }
    text.to_string()
}

fn valid_dice(dice: &str) -> bool {
    let part = |part: &str| -> bool {
        match part.split_once('d') {
            Some((count, sides)) => {
                (count.is_empty() || count.chars().all(|c| c.is_ascii_digit()))
                    && !sides.is_empty()
                    && sides.chars().all(|c| c.is_ascii_digit())
            }
            None => !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
        }
    };
    let mut current = String::new();
    let mut parts = Vec::new();
    for c in dice.chars() {
        if c == '+' || c == '-' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts.iter().all(|p| part(p))
}

impl App {
    pub fn run_command(&mut self, text: &str) {
        let body = &text[1..];
        let (name, args) = body.split_once(' ').unwrap_or((body, ""));
        let name = resolve_alias(name);
        let args = args.trim();
        match name.as_str() {
            "join" => match args {
                "" => self.warn("Usage: /join <channel>"),
                _ => self.join(&unquote(args), true),
            },
            "leave" => self.close_chat(),
            "priv" => match args {
                "" => self.warn("Usage: /priv <character>"),
                _ => self.open_private(&unquote(args)),
            },
            "me" => {
                if let Some(chat) = self.current_conversation() {
                    if !args.is_empty() {
                        self.send_message(&chat, &format!("/me {args}"));
                    }
                }
            }
            "ad" => self.cmd_ad(args),
            "invite" => self.cmd_invite(args),
            "code" => {
                if let Some(chat) = self.current_channel() {
                    let title = self.store.get(&chat).map(|c| c.title.clone()).unwrap_or_default();
                    let key = self.store.get(&chat).map(|c| c.key.clone()).unwrap_or_default();
                    let code = format!("[session={}]{}[/session]", html_unescape(&title), key);
                    self.copy_to_clipboard(&code);
                    self.notify_titled("Channel code copied to clipboard", code, Severity::Information, 15);
                }
            }
            "makeroom" => match unquote(args).as_str() {
                "" => self.warn("Usage: /makeroom <title>"),
                title => self.create_room(title),
            },
            "openroom" => self.channel_state("public"),
            "closeroom" => self.channel_state("private"),
            "kick" => self.channel_name_command(opcode::KICK, args, "/kick <name>"),
            "ban" => self.channel_name_command(opcode::BAN, args, "/ban <name>"),
            "unban" => self.channel_name_command(opcode::UNBAN, args, "/unban <name>"),
            "op" => self.channel_name_command(opcode::PROMOTE_OP, args, "/op <name>"),
            "deop" => self.channel_name_command(opcode::DEMOTE_OP, args, "/deop <name>"),
            "setowner" => self.channel_name_command(opcode::SET_CHANNEL_OWNER, args, "/setowner <name>"),
            "banlist" => {
                if let Some(key) = self.current_channel_key() {
                    self.send(opcode::BANLIST, Some(json!({"channel": key})));
                }
            }
            "timeout" => self.cmd_timeout(args),
            "setdescription" => match (args, self.current_channel_key()) {
                ("", Some(_)) => self.warn("Usage: /setdescription <text>"),
                (text, Some(key)) => self.send(opcode::CHANNEL_DESCRIPTION, Some(json!({"channel": key, "description": text}))),
                _ => {}
            },
            "setmode" => {
                let mode = args.to_lowercase();
                if let Some(key) = self.current_channel_key() {
                    if ["chat", "ads", "both"].contains(&mode.as_str()) {
                        self.send(opcode::SET_CHANNEL_MODE, Some(json!({"channel": key, "mode": mode})));
                    } else {
                        self.warn("Usage: /setmode <chat|ads|both>");
                    }
                }
            }
            "roll" => {
                if let Some(chat) = self.current_conversation() {
                    let dice = args.replace(' ', "").to_lowercase();
                    let dice = if dice.is_empty() { "1d6".to_string() } else { dice };
                    if valid_dice(&dice) {
                        self.roll(&chat, &dice);
                    } else {
                        self.warn(format!("Invalid dice '{args}', e.g. /roll 1d20 or /roll 2d6+3"));
                    }
                }
            }
            "bottle" => match self.current_chat().filter(|chat| chat.kind == CHANNEL).map(|chat| chat.id()) {
                Some(id) => self.roll(&id, "bottle"),
                None => self.warn("/bottle only works in channels"),
            },
            "pin" => self.cmd_pin(),
            "unpin" => self.cmd_unpin(),
            "pins" => {
                if self.pins.items.is_empty() {
                    self.notify("No pinned chats", Severity::Information);
                } else {
                    let lines: Vec<String> = self
                        .pins
                        .items
                        .iter()
                        .map(|pin| format!("{}{}", if pin.kind == CHANNEL { '#' } else { '@' }, pin.title.clone().unwrap_or_else(|| pin.key.clone())))
                        .collect();
                    self.notify_titled("Pinned", lines.join("\n"), Severity::Information, 10);
                }
            }
            "ads" => self.cmd_ads(args),
            "char" => self.cmd_char(args),
            "reconnect" => {
                if self.online {
                    self.notify("Already connected", Severity::Information);
                } else {
                    self.schedule_reconnect(true);
                }
            }
            "logout" => self.logout(),
            "minimize" => self.minimize_requested = true,
            "users" => self.show_sidebar = !self.show_sidebar,
            "status" => self.cmd_status(args),
            "status:flag" => self.cmd_status_flag(args),
            "status:message" => {
                if args.is_empty() {
                    let screen = EditScreen::new("Status message", &self.own_status.message, MAX_STATUS_MESSAGE);
                    self.screens.push(Screen::Edit(screen));
                } else {
                    let status = self.own_status.status.clone();
                    self.apply_status(&status, args);
                }
            }
            "channels" => self.open_channel_list(),
            "bbcode" => self.open_picker(PickerKind::Bbcode, args),
            "emoji" => self.open_picker(PickerKind::Emoji, args),
            "profile" => self.cmd_profile(args),
            "chatstatus" => self.cmd_chatstatus(),
            "help" => self.open_picker(PickerKind::Commands, ""),
            "shortcuts" => {
                let lines: Vec<String> = SHORTCUT_INFO.iter().map(|(key, text)| format!("{key:<22} {text}")).collect();
                self.show_info("Shortcuts", text_lines(&lines.join("\n"), Default::default()), true);
            }
            "quit" => self.should_quit = true,
            _ => self.warn(format!("Unknown command: /{name}  (see /help)")),
        }
    }

    pub fn open_picker(&mut self, kind: PickerKind, filter: &str) {
        let (title, items) = match kind {
            PickerKind::Emoji => ("Emoji", emoji_items()),
            PickerKind::Bbcode => ("BBCode", bbcode_items()),
            PickerKind::Channels => ("Channels", self.channel_items()),
            PickerKind::Commands => (
                "Commands",
                COMMAND_INFO
                    .iter()
                    .map(|(name, args, description)| PickItem {
                        label: format!("{:<26} {description}", command_usage(name, args)),
                        value: format!("/{name} "),
                        hint: name.to_string(),
                    })
                    .collect(),
            ),
        };
        self.screens.push(Screen::Picker(PickerScreen::new(kind, title, items, filter)));
    }

    fn channel_items(&self) -> Vec<PickItem> {
        let mut items = Vec::new();
        for (code, name) in &self.official {
            items.push(PickItem { label: format!("#{name}"), value: code.clone(), hint: "official".into() });
        }
        for (code, title) in &self.open_private {
            if self.official.contains_key(code) {
                continue;
            }
            items.push(PickItem { label: format!("#{} (private)", html_unescape(title)), value: code.clone(), hint: code.clone() });
        }
        items.sort_by_key(|item| item.label.to_lowercase());
        items
    }

    pub fn open_channel_list(&mut self) {
        if self.official.is_empty() && self.open_private.is_empty() {
            self.send(opcode::LIST_OFFICIAL_CHANNELS, None);
            self.send(opcode::LIST_PRIVATE_CHANNELS, None);
            self.warn("Channel list is loading, try again in a moment");
            return;
        }
        self.open_picker(PickerKind::Channels, "");
    }

    fn current_conversation(&mut self) -> Option<String> {
        match self.current_chat().filter(|chat| chat.kind != CONSOLE).map(|chat| chat.id()) {
            Some(id) => Some(id),
            None => {
                self.warn("Open a channel or private chat first");
                None
            }
        }
    }

    fn current_channel(&mut self) -> Option<String> {
        match self.current_chat().filter(|chat| chat.kind == CHANNEL).map(|chat| chat.id()) {
            Some(id) => Some(id),
            None => {
                self.warn("This command only works in a channel");
                None
            }
        }
    }

    fn current_channel_key(&mut self) -> Option<String> {
        let id = self.current_channel()?;
        self.store.get(&id).map(|chat| chat.key.clone())
    }

    pub fn cmd_ad(&mut self, args: &str) {
        let Some(id) = self.current_channel() else { return };
        let message = args.trim();
        if message.is_empty() {
            self.warn("Usage: /ad <text>");
            return;
        }
        let limit = self.variables.get("lfrp_max").and_then(Value::as_u64).unwrap_or(50000) as usize;
        let length = message.chars().count();
        if length > limit {
            self.warn(format!("Ad too long ({length}/{limit})"));
            return;
        }
        let (key, title) = self.store.get(&id).map(|chat| (chat.key.clone(), chat.title.clone())).unwrap_or_default();
        let wait = self.ad_wait_seconds(&key);
        if wait > 0 {
            self.warn(format!("You can post the next ad in {title} in {}:{:02} min", wait / 60, wait % 60));
            return;
        }
        self.send_ad(&id, message);
    }

    fn cmd_invite(&mut self, args: &str) {
        let Some(id) = self.current_channel() else {
            return;
        };
        let name = unquote(args);
        if name.is_empty() {
            self.warn("Usage: /invite <character>");
            return;
        }
        let name = self.find_online(&name).unwrap_or(name);
        let (key, title) = self.store.get(&id).map(|chat| (chat.key.clone(), chat.title.clone())).unwrap_or_default();
        self.invite(&key, &name);
        self.notify(format!("Invited {name} to {title}"), Severity::Information);
    }

    fn channel_name_command(&mut self, op: &str, args: &str, usage: &str) {
        let Some(key) = self.current_channel_key() else { return };
        let name = unquote(args);
        if name.is_empty() {
            self.warn(format!("Usage: {usage}"));
            return;
        }
        let name = self.find_online(&name).unwrap_or(name);
        self.send(op, Some(json!({"channel": key, "character": name})));
    }

    fn channel_state(&mut self, state: &str) {
        if let Some(key) = self.current_channel_key() {
            self.send(opcode::SET_PRIVATE_CHANNEL_STATE, Some(json!({"channel": key, "status": state})));
        }
    }

    fn cmd_timeout(&mut self, args: &str) {
        let Some(key) = self.current_channel_key() else { return };
        let (name_part, minutes) = args.trim().rsplit_once(' ').unwrap_or(("", ""));
        let name = unquote(name_part);
        let minutes: u32 = minutes.parse().unwrap_or(0);
        if name.is_empty() || minutes < 1 {
            self.warn("Usage: /timeout <name> <minutes>");
            return;
        }
        let name = self.find_online(&name).unwrap_or(name);
        self.send(opcode::TIMEOUT, Some(json!({"channel": key, "character": name, "length": minutes})));
    }

    fn cmd_pin(&mut self) {
        let Some(id) = self.current_conversation() else { return };
        let Some(chat) = self.store.get(&id).cloned() else { return };
        if self.pins.pin(chat.kind, &chat.key, &chat.title) {
            self.notify(format!("Pinned {}", chat.title), Severity::Information);
        }
    }

    fn cmd_unpin(&mut self) {
        let Some(chat) = self.current_chat().cloned() else {
            self.warn("No chat open");
            return;
        };
        if self.pins.unpin(chat.kind, &chat.key) {
            self.notify(format!("Unpinned {}", chat.title), Severity::Information);
        }
    }

    fn cmd_ads(&mut self, args: &str) {
        let Some(id) = self.current_channel() else { return };
        let Some(chat) = self.store.get(&id).cloned() else { return };
        let value = args.trim().to_lowercase();
        if !["", "on", "off"].contains(&value.as_str()) {
            self.warn("Usage: /ads [on|off]");
            return;
        }
        if !value.is_empty() {
            self.ads.set_visible(&chat.key, value == "on");
            self.open_chat(&id);
        }
        let state = if self.ads.visible(&chat.key) { "shown" } else { "hidden" };
        self.notify(format!("Ads in {}: {state}", chat.title), Severity::Information);
    }

    fn cmd_char(&mut self, args: &str) {
        if self.characters.is_empty() {
            self.warn("Character list not available yet");
            return;
        }
        let name = unquote(args);
        if name.is_empty() {
            let preferred = self.character.clone();
            self.choose_character(&preferred);
            return;
        }
        match self.match_character(&name) {
            None => self.warn(format!("'{name}' is not a character of this account")),
            Some(found) if found == self.character => self.notify(format!("Already logged in as {found}"), Severity::Information),
            Some(found) => self.on_character_chosen(Some(found)),
        }
    }

    fn cmd_status(&mut self, args: &str) {
        if args.is_empty() {
            let message = if self.own_status.message.is_empty() { String::new() } else { format!(": {}", self.own_status.message) };
            let text = format!("Status {}{message}\nUsage: /status [{}] [message]", self.own_status.status, VALID_STATUSES.join("|"));
            self.notify_titled("Status", text, Severity::Information, 8);
            return;
        }
        let (first, rest) = args.split_once(' ').unwrap_or((args, ""));
        if VALID_STATUSES.contains(&first.to_lowercase().as_str()) {
            self.apply_status(&first.to_lowercase(), rest.trim());
        } else {
            let status = self.own_status.status.clone();
            self.apply_status(&status, args.trim());
        }
    }

    fn cmd_status_flag(&mut self, args: &str) {
        let status = args.trim().to_lowercase();
        if status.is_empty() {
            let text = format!("Status {}\nUsage: /status:flag <{}>", self.own_status.status, VALID_STATUSES.join("|"));
            self.notify_titled("Status", text, Severity::Information, 8);
        } else if !VALID_STATUSES.contains(&status.as_str()) {
            self.warn(format!("Unknown status '{status}', use one of: {}", VALID_STATUSES.join(", ")));
        } else {
            let message = self.own_status.message.clone();
            self.apply_status(&status, &message);
        }
    }

    pub fn apply_status(&mut self, status: &str, message: &str) {
        let length = message.chars().count();
        if length > MAX_STATUS_MESSAGE {
            self.warn(format!("Status message too long ({length}/{MAX_STATUS_MESSAGE})"));
            return;
        }
        self.own_status.set(status, message);
        if self.online {
            self.send_status();
            self.update_subtitle();
        }
        let suffix = if message.is_empty() { String::new() } else { format!(": {message}") };
        self.notify(format!("Status set to {status}{suffix}"), Severity::Information);
    }

    fn cmd_profile(&mut self, args: &str) {
        let mut name = unquote(args);
        if name.is_empty() {
            match self.current_chat().filter(|chat| chat.kind == USER) {
                Some(chat) => name = chat.key.clone(),
                None => {
                    self.warn("Usage: /profile <name> (without a name in a private chat)");
                    return;
                }
            }
        }
        let name = self.find_online(&name).unwrap_or(name);
        self.show_profile(&name);
    }

    fn cmd_chatstatus(&mut self) {
        let Some(chat) = self.current_chat().filter(|chat| chat.kind != CONSOLE).cloned() else {
            self.warn("Open a private chat or a channel first");
            return;
        };
        let title = html_unescape(&chat.title);
        let mut lines: Vec<Vec<Seg>> = Vec::new();
        let no_names = |_: &str| ratatui::style::Style::default();
        if chat.kind == CHANNEL {
            lines.push(vec![Seg::new(title.clone(), bold())]);
            let count = self.presence.channel_members(&chat.key).map(|members| format!("  \u{b7}  {} users", members.len())).unwrap_or_default();
            lines.push(vec![Seg::new(format!("{}{count}", chat.key), dim())]);
            lines.push(Vec::new());
            let description = self.descriptions.get(&chat.key).cloned().unwrap_or_default();
            if description.trim().is_empty() {
                lines.push(vec![Seg::new("(no description)", dim())]);
            } else {
                lines.extend(crate::bbcode::split_lines(&to_segs(&description, &no_names, true)));
            }
            self.show_info("Channel description", lines, true);
            return;
        }
        lines.push(vec![Seg::new(title, self.name_style(&chat.key).patch(bold()))]);
        match self.presence.status_of(&chat.key) {
            None => lines.push(vec![Seg::new("offline", dim())]),
            Some((status, message)) => {
                lines.push(vec![Seg::new(status.clone(), status_style(&status))]);
                lines.push(Vec::new());
                if message.trim().is_empty() {
                    lines.push(vec![Seg::new("(no status message)", dim())]);
                } else {
                    lines.extend(crate::bbcode::split_lines(&to_segs(&message, &no_names, true)));
                }
            }
        }
        self.show_info("Status", lines, true);
    }
}
