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
| `terminalchat` | tmux launcher (`/minimize` detaches, client keeps running) |
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
```

## Locations

| Purpose | Default | Override |
|---|---|---|
| Configuration | `~/.config/terminalchat/config.json` | `TERMINALCHAT_CONFIG` |
| Data | `~/.local/share/terminalchat/` | `TERMINALCHAT_DATA`, then `data_dir` in the config |
| Cache | `~/.cache/terminalchat/` | `XDG_CACHE_HOME` |

Unknown keys in `config.json` are kept when the file is saved.

## Run

```bash
./terminalchat      # starts inside tmux; /minimize or ctrl+t detaches
```

Without a complete `config.json` a login screen appears. `/help` lists commands, `/shortcuts` the key bindings. Hold Shift to select text with the mouse.

## Images

- Needs `"eicons": "inline"` in `config.json` and a terminal with graphics (Sixel, Kitty or iTerm2). The terminal is asked at start; inside tmux the launcher enables `allow-passthrough`.
- `TERMINALCHAT_IMAGES=off|sixel|kitty|iterm2` overrides the detection. The `terminalchat` launcher sets `sixel` unless the variable is already set. A tmux session that is already running keeps its old environment: stop it first (`tmux -L terminalchat kill-server`).
- In tmux the images are sent past tmux straight to the terminal. `TERMINALCHAT_TMUX_NATIVE=1` lets tmux draw them instead: it then places and clears them itself, but scales them down and redraws them more slowly.
- Eicons and icons of a message appear in a strip below the message (`eicon_height` rows). Click an image to copy its link. The profile shows the avatar on top and thumbnails in the Images section; click a thumbnail to copy the link of the full image.
- `/gif [on|off]` switches GIF animation (config `eicon_animate`), The cell size is measured once at start; if the measurement is wrong (SSH, tmux), set `cell_size` in config.json, e.g. `"8x18"` (pixels per cell). After zooming the font, restart the client. Animation runs only for visible images and pauses while the tmux client is detached.
- Cache: `~/.cache/terminalchat/{eicons,avatars,thumbs}` (same as the Python version), refreshed after `eicon_max_age_hours`.

## GitHub Actions

`.github/workflows/ci.yml` runs clippy and the tests on x86_64 and ARM64 for every push and pull request. A tag like `v0.1.0` additionally builds both binaries and attaches `terminalchat-x86_64` and `terminalchat-aarch64` to a GitHub release (built on Ubuntu 22.04, so they also run on Raspberry Pi OS Bookworm). On the Pi, `./setup.sh --download` fetches the binary of the latest release instead of building.

