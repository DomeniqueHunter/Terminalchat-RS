# terminalchat (Rust)

Rust rewrite of [terminalchat](../Terminalchat), a terminal client for [F-Chat](https://www.f-list.net/chat3/).
It uses the same `config.json` and the same data directories as the Python version, so both can be used on the same data.

Status: complete client (chat, channels, private chats, ads, moderation, profile, pickers, reconnect). Not ported: inline eicon/avatar images (links are shown as text).

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
