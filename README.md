# terminalchat (Rust)

Rust rewrite of [terminalchat](../Terminalchat), a terminal client for [F-Chat](https://www.f-list.net/chat3/).
It uses the same `config.json` and the same data directories as the Python version, so both can be used on the same data.

Status: complete client (chat, channels, private chats, ads, moderation, profile, pickers, reconnect). Images (eicons, avatars, profile thumbnails, GIF animation) are drawn inline when the terminal supports Sixel, Kitty or iTerm2 graphics; otherwise links are shown as text.

## Layout

| Path | Content |
|---|---|
| `crates/fchat` | Protocol library: opcodes, login. See its README |
| `crates/terminalchat` | The client: config, paths, later store, BBCode, commands, TUI |
| `setup.sh` | Installs tmux, C compiler and Rust (rustup) and builds for this machine |
| `purge.sh` | Deletes config, data, logs, cache (and with `--build` target/ and binaries) |
| `usage.sh` | One-shot: date, PID, RSS, CPU%, MEM%, runtime of the running client |
| `bench.sh` | Measures memory and CPU of the running client (also `--python`), one line per sample with date and time |
| `build.sh` | Release build (`native`, `arm64`, `all`) and `clean` |
| `terminalchat` | tmux launcher (`/minimize` detaches, client keeps running); own tmux server `terminalchat-rs`, separate from the Python version |
| `tmux.conf` | tmux settings used by the launcher |
| `build/` | Output of `build.sh` |

## Install

```bash
./setup.sh          # packages, Rust, build, link to ~/.local/bin/terminalchat; -y: no questions
./purge.sh -n       # show what would be deleted (shared with the Python version!)
```

## Build

```bash
cargo test
./build.sh          # native
./build.sh clean    # remove target/ and build/*
./build.sh arm64    # cross build, needs aarch64-linux-gnu-gcc; or run ./build.sh on the Pi itself
./build.sh windows  # cross build of build/terminalchat-x86_64.exe, needs gcc-mingw-w64-x86-64
./build.sh --restart  # native build, then restart the client with the new binary
```

`--restart` ends a running client cleanly (SIGTERM, it saves its files like `/quit`) and starts the new binary in the same tmux pane, so the attached window shows the new version; without a running session it starts `./terminalchat` in the current terminal. Nothing restarts when the build fails.

## Locations

| Purpose | Default | Override |
|---|---|---|
| Configuration | `~/.config/terminalchat/config.json` | `TERMINALCHAT_CONFIG` |
| Data | `~/.local/share/terminalchat/` | `TERMINALCHAT_DATA`, then `data_dir` in the config |
| Cache | `~/.cache/terminalchat/` | `XDG_CACHE_HOME` |

On Windows: configuration `%APPDATA%\terminalchat\config.json`, data `%LOCALAPPDATA%\terminalchat\`, cache `%LOCALAPPDATA%\terminalchat\cache\`.

Unknown keys in `config.json` are kept when the file is saved.

## Run

```bash
./terminalchat      # starts inside tmux; /minimize or ctrl+t detaches
```

Without a complete `config.json` a login screen appears. `/help` lists commands, `/shortcuts` the key bindings. Hold Shift to select text with the mouse.

## Windows

`terminalchat-x86_64.exe` runs directly, without tmux (so no `/minimize`, ctrl+t). Use Windows Terminal: images need Sixel, which it has since version 1.22; the old console window shows no images. Windows Terminal does not answer the graphics query, so on Windows the client uses Sixel when the query fails; if the eicons have the wrong size, set `cell_size` in config.json (e.g. `"10x20"`). If images hang or show garbage, start with `set TERMINALCHAT_IMAGES=off` (cmd) or `$env:TERMINALCHAT_IMAGES="off"` (PowerShell). Closing the window ends the client like `/quit`.

## Images

- Needs `"eicons": "inline"` in `config.json` and a terminal with graphics (Sixel, Kitty or iTerm2). The terminal is asked at start; inside tmux the launcher enables `allow-passthrough`.
- `TERMINALCHAT_IMAGES=off|sixel|kitty|iterm2` overrides the detection. The `terminalchat` launcher sets `sixel` unless the variable is already set. A tmux session that is already running keeps its old environment: stop it first (`./kill-tmux.sh`).
- In tmux the images are sent past tmux straight to the terminal. `TERMINALCHAT_TMUX_NATIVE=1` lets tmux draw them instead: it then places and clears them itself, but scales them down and redraws them more slowly.
- Eicons and icons of a message appear in a strip below the message (`eicon_height` rows). Click an image to copy its link. The profile (`/profile`) looks like in the Python version: a header with avatar, name and title, status, dates and link, below it the tabs Info, Description, Kinks (four columns) and Images (a grid of up to five thumbnails with their descriptions). Switch tabs with left/right, tab or a click, scroll with up/down; `o` copies the profile link, a click on a thumbnail the link of the full image. Images load when their tab is opened.
- `/gif [on|off]` switches GIF animation (config `eicon_animate`), The cell size is measured once at start; if the measurement is wrong (SSH, tmux), set `cell_size` in config.json, e.g. `"8x18"` (pixels per cell). After zooming the font, restart the client. Animation runs only for visible images and pauses while the tmux client is detached.
- Cache: `~/.cache/terminalchat/{eicons,avatars,thumbs}` (same as the Python version), refreshed after `eicon_max_age_hours`.


## Settings

`/settings` opens the settings page with two tabs (switch with `tab`):

- **General**: every entry of `config.json` except account, password and character (those go through the login and `/char`). Switches and choices change with `enter` or `space`, other values are edited in the row. Entries that count only later say so: connection after `/reconnect`, `default_channels` at the next login, `eicons`, `cell_size` and `data_dir` after a restart. A changed `data_dir` does not move existing data.
- **Colors**: all entries of the block `colors` with a preview. `enter` opens the editor: the value as text, a color picker for color and background (`left`/`right`, `none` for no color), check boxes for bold, dim, italic, underline, strike and reverse (`space`), and a preview. Text and picker follow each other.

The mouse works too: a click on a tab switches, a click on a row chooses it and a second click changes it; in the color editor a click picks a swatch, toggles a check box or focuses a part.

Every accepted change is used at once and saved to `config.json` right away; a value that cannot be read is refused with the reason. `r` sets an entry back to its default. The `[Reset]` button at the top right (or `R`) sets everything of the current tab back to default after a confirmation (`y` or a second click): on *General* all general settings, on *Colors* all colors. Colors equal to the default are not stored.

## Ping

`ping` in `/settings` (General, off by default) rings the terminal bell for every incoming message, private or in a channel, at most once a second. The terminal decides what the bell does (sound, flash or nothing; in Konsole: profile, *Terminal Bell*). `/mute` switches the ping off for the current chat, `/mute <chat>` for a named one (channel or character), and the same command again switches it back on. `/muteall` mutes all open chats, again: unmutes them. Muted chats are kept in `muted.json` per character.

## Notes

`/note` (or `/notes`, `ctrl+n`) opens the F-List notes, the mails of the website, for all characters of the account at once:

- Tabs **Inbox** and **Sent** (left/right, tab or a click), each with From and To, so you see to which of your characters a note went. More notes load when the selection reaches the end; `f5` reloads.
- `enter` or a second click reads a note (BBCode, links clickable), `esc` goes back.
- `r` answers: the sender is the character that got the note, the recipient the one who wrote it, the title `Re: ...`. `n` writes a new note, sent by the chat character. In the draft `tab` walks the fields, left/right in *From* picks any character of the account, `ctrl+j` (or `ctrl+enter`) is a new line, `enter` in the message sends, `esc` cancels (asks first if there is text).
- `d` moves a note to the trash after a confirmation (`y`).

There is no API for notes; the client logs in on the website with the account and password of the chat (kept in memory only, no cookies on disk) and uses the same calls as the website. If F-List changes the website, this may break.

## Colors

All colors can be changed in the block `colors` of `config.json`. Only the entries that differ from the defaults are needed; the Python version keeps the block when it saves the config. A style is written like in the Python version: `bold`, `dim`, `italic`, `underline`, `strike`, `reverse`, a color `#rrggbb` and a background `on #rrggbb`. Entries that cannot be read are reported at start and keep their default.

Backgrounds (`#rrggbb`, or `""` for the terminal's own background, e.g. to keep a transparent background). Like the Python version every cell gets an explicit background by default; with the terminal background Konsole may leave pieces of images that are gone standing in empty cells.

```json
"colors": {
    "background": "#121212",
    "surface": "#1e1e1e",
    "panel": "#242f38",
    "status_box": "#121212",
    "status_field": "#121212",
    "settings_box": "#121212",
    "settings_field": "#1e1e1e",

    "user": "bold #008080",
    "icon": "bold #008080",
    "eicon": "#af5fd7",
    "url": "underline #5f87ff",
    "sender": "bold",
    "spoiler": "reverse",
    "op": "dim",
    "channel_link": "bold #5fafff",
    "rule": "dim",
    "quote": "dim",
    "online": "#5fd75f",
    "offline": "#d75f5f",
    "focus": "#5f87d7",
    "information": "#5f87d7",
    "warning": "#d7af5f",
    "error": "#d75f5f",
    "ad": "#d7af5f",
    "footer": "on #1f2d4a",
    "footer_key": "bold #ffb000",
    "settings_selected": "reverse",
    "settings_group": "bold",

    "status": {"looking": "#55ff55", "online": "#008080", "idle": "#a8a8a8", "away": "#a8a8a8", "busy": "#ffaf00", "dnd": "#ff5555", "crown": "#ffff55"},
    "gender": {"male": "#6699ff", "female": "#ff6699", "transgender": "#ee8822", "herm": "#9b30ff", "male-herm": "#007fff", "cunt-boy": "#00cc66", "shemale": "#cc66ff", "none": "#ffffbb"},
    "bbcode": {"red": "#ff5555", "blue": "#5f87ff", "white": "#ffffff", "yellow": "#ffff55", "pink": "#ff87d7", "gray": "#a8a8a8", "green": "#55ff55", "orange": "#ffaf00", "purple": "#af5fff", "black": "#5f5f5f", "brown": "#af8700", "cyan": "#55ffff"}
}
```

`surface` is the input field, `panel` popups and toasts, `status_box` and `status_field` the status message box and its text field, `settings_box`, `settings_field`, `settings_selected` and `settings_group` the settings page, its edit field and color editor, the chosen row and tab, and the group titles, `focus` the border of the focused panel, `ad` the input border in ad mode, `footer` the key bar. `NO_COLOR` turns all colors off but keeps the attributes.

## GitHub Actions

`.github/workflows/ci.yml` runs clippy and the tests on x86_64, ARM64 and Windows for every push to main and every pull request; other branches can be run by hand (Actions, *Run workflow*). A tag like `v0.1.0` additionally builds the binaries and attaches `terminalchat-x86_64`, `terminalchat-aarch64` and `terminalchat-x86_64.exe` to a GitHub release (built on Ubuntu 22.04, so they also run on Raspberry Pi OS Bookworm). On the Pi, `./setup.sh --download` fetches the binary of the latest release instead of building.

