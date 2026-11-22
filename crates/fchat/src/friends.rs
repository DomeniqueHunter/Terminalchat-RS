//! Friends, friend requests and bookmarks of the account, through the JSON API of f-list.net
//! (`friend-list.php`, `request-*.php`, `bookmark-*.php`). Friendships are between two
//! characters; bookmarks belong to the account.

use serde_json::Value;

use crate::error::Result;
use crate::profile::request;

/// One friendship: `own` is a character of the account, `other` the friend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Friendship {
    pub own: String,
    pub other: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FriendRequest {
    pub id: u64,
    pub source: String,
    pub dest: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FriendData {
    pub friends: Vec<Friendship>,
    /// Requests from others to characters of the account.
    pub incoming: Vec<FriendRequest>,
    /// Requests of the account that wait for an answer.
    pub outgoing: Vec<FriendRequest>,
    pub bookmarks: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accept,
    Deny,
    /// Takes back an own request.
    Cancel,
}

/// A name, given as text or as an object with a "name".
fn name(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Object(map)) => map.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
        _ => String::new(),
    }
}

/// The first of several keys that holds a name; the API is not documented and differs between calls.
fn first_name(item: &Value, keys: &[&str]) -> String {
    keys.iter().map(|key| name(item.get(key))).find(|text| !text.is_empty()).unwrap_or_default()
}

fn number(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

const SOURCE: [&str; 4] = ["source", "source_name", "sourceName", "character"];
const DEST: [&str; 5] = ["dest", "dest_name", "target", "target_name", "destName"];

fn items<'a>(data: &'a Value, keys: &[&str]) -> &'a [Value] {
    keys.iter().find_map(|key| data.get(key).and_then(Value::as_array)).map(Vec::as_slice).unwrap_or(&[])
}

/// Friendships from `friend-list.php`; `own` tells which side is a character of the account.
pub fn parse_friends(data: &Value, own: &[String]) -> Vec<Friendship> {
    let is_own = |name: &str| own.iter().any(|character| character.eq_ignore_ascii_case(name));
    let mut friends: Vec<Friendship> = items(data, &["friends"])
        .iter()
        .filter_map(|item| {
            let (source, dest) = (first_name(item, &SOURCE), first_name(item, &DEST));
            if source.is_empty() || dest.is_empty() {
                return None;
            }
            Some(if !is_own(&source) && is_own(&dest) { Friendship { own: dest, other: source } } else { Friendship { own: source, other: dest } })
        })
        .collect();
    friends.sort_by_key(|friend| (friend.other.to_lowercase(), friend.own.to_lowercase()));
    friends
}

pub fn parse_requests(data: &Value) -> Vec<FriendRequest> {
    items(data, &["requests", "friends"])
        .iter()
        .map(|item| FriendRequest {
            id: number(item.get("id").or_else(|| item.get("request_id"))),
            source: first_name(item, &SOURCE),
            dest: first_name(item, &DEST),
        })
        .filter(|request| request.id != 0)
        .collect()
}

pub fn parse_bookmarks(data: &Value) -> Vec<String> {
    let mut names: Vec<String> = items(data, &["characters", "bookmarks"]).iter().map(|item| name(Some(item))).filter(|name| !name.is_empty()).collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// Everything at once: four calls.
pub async fn load(account: &str, ticket: &str, own: &[String]) -> Result<FriendData> {
    let auth = [("account", account), ("ticket", ticket)];
    let friends = parse_friends(&request("friend-list.php", &auth).await?, own);
    let incoming = parse_requests(&request("request-list.php", &auth).await?);
    let outgoing = parse_requests(&request("request-pending.php", &auth).await?);
    let bookmarks = parse_bookmarks(&request("bookmark-list.php", &auth).await?);
    Ok(FriendData { friends, incoming, outgoing, bookmarks })
}

pub async fn bookmark(account: &str, ticket: &str, name: &str, add: bool) -> Result<()> {
    let endpoint = if add { "bookmark-add.php" } else { "bookmark-remove.php" };
    request(endpoint, &[("account", account), ("ticket", ticket), ("name", name)]).await.map(|_| ())
}

pub async fn send_request(account: &str, ticket: &str, source: &str, dest: &str) -> Result<()> {
    request("request-send.php", &[("account", account), ("ticket", ticket), ("source_name", source), ("dest_name", dest)]).await.map(|_| ())
}

pub async fn answer(account: &str, ticket: &str, id: u64, answer: Answer) -> Result<()> {
    let endpoint = match answer {
        Answer::Accept => "request-accept.php",
        Answer::Deny => "request-deny.php",
        Answer::Cancel => "request-cancel.php",
    };
    let id = id.to_string();
    request(endpoint, &[("account", account), ("ticket", ticket), ("request_id", id.as_str())]).await.map(|_| ())
}

pub async fn remove_friend(account: &str, ticket: &str, own: &str, other: &str) -> Result<()> {
    request("friend-remove.php", &[("account", account), ("ticket", ticket), ("source_name", own), ("dest_name", other)]).await.map(|_| ())
}


#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;


    #[test]
    fn friendships_put_the_own_character_first() {
        let own = vec!["Anna".to_string(), "Bo".to_string()];
        let data = json!({"friends": [
            {"source": "Zed", "dest": "anna", "last_online": 5},
            {"source_name": "Bo", "dest_name": "Yan"},
            {"source": {"name": "Bo"}, "target": {"name": "Abe"}},
            {"source": "", "dest": "X"}
        ]});
        let friends = parse_friends(&data, &own);
        let pairs: Vec<(&str, &str)> = friends.iter().map(|f| (f.own.as_str(), f.other.as_str())).collect();
        assert_eq!(pairs, vec![("Bo", "Abe"), ("Bo", "Yan"), ("anna", "Zed")]);
    }


    #[test]
    fn requests_and_bookmarks_are_read_whatever_the_field_names() {
        let data = json!({"requests": [
            {"id": "12", "source": "Zed", "dest": "Anna"},
            {"request_id": 13, "source_name": "Bo", "target": "Yan"},
            {"source": "no id", "dest": "x"}
        ]});
        assert_eq!(
            parse_requests(&data),
            vec![
                FriendRequest { id: 12, source: "Zed".into(), dest: "Anna".into() },
                FriendRequest { id: 13, source: "Bo".into(), dest: "Yan".into() }
            ]
        );
        assert_eq!(parse_bookmarks(&json!({"characters": ["zed", {"name": "Abe"}]})), vec!["Abe", "zed"]);
        assert!(parse_requests(&json!({"error": ""})).is_empty());
    }
}
