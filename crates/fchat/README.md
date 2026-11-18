# fchat

Protocol library for the F-Chat chat server of f-list.net. It has no UI and no client logic, so it can be used by any program.

| Module | Content |
|---|---|
| `opcode` | All three letter operation codes as `&str` constants |
| `login` | `fetch_ticket(account, password)`: API ticket, character list, bookmarks and friends; `parse_login` for the raw JSON |
| `connection` | Websocket: `Connection::connect`, 1 message/s send queue, priority sends (PIN), events as channel |
| `profile` | `fetch_profile` (character data + cached kink mapping) |
| `error` | `Error::Rejected` (do not retry) and `Error::Temporary` (retry later) |

```rust
let result = fchat::fetch_ticket("account", "password").await?;
println!("{} characters", result.characters.len());
```
