//! Login against the f-list.net JSON API: fetches the API ticket together with the
//! character list, bookmarks and friends of the account.

use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};

pub const API_TICKET_URL: &str = "https://www.f-list.net/json/getApiTicket.php";
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginResult {
    pub ticket: String,
    pub characters: Vec<String>,
    pub default_character: String,
    pub bookmarks: Vec<String>,
    /// Pairs of (source character, destination character); `None` if the server sent no list.
    pub friends: Option<Vec<(String, String)>>,
}

/// Asks the server for a ticket. Missing credentials count as a rejected login.
pub async fn fetch_ticket(account: &str, password: &str) -> Result<LoginResult> {
    if account.is_empty() || password.is_empty() {
        return Err(Error::Rejected("Account and password are required".into()));
    }

    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Error::Temporary(format!("Login request failed: {error}")))?;
    let form = [
        ("account", account),
        ("password", password),
        ("no_friends", "false"),
        ("no_bookmarks", "false"),
        ("new_character_list", "true"),
    ];
    let response = client
        .post(API_TICKET_URL)
        .form(&form)
        .send()
        .await
        .map_err(|error| Error::Temporary(format!("Login request failed: {error}")))?;

    if !response.status().is_success() {
        let message = format!("Login server answered with HTTP {}", response.status().as_u16());
        return Err(Error::Temporary(message));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| Error::Temporary("Unexpected answer from login server".into()))?;
    parse_login(&payload)
}

/// Turns the JSON answer of `getApiTicket.php` into a [`LoginResult`].
pub fn parse_login(payload: &Value) -> Result<LoginResult> {
    let Some(object) = payload.as_object() else {
        return Err(Error::Temporary("Unexpected answer from login server".into()));
    };
    if let Some(message) = object.get("error").and_then(Value::as_str).filter(|text| !text.is_empty()) {
        return Err(Error::Rejected(message.to_string()));
    }

    let mut characters = match object.get("characters") {
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        Some(Value::Array(items)) => items.iter().filter_map(value_text).collect(),
        _ => Vec::new(),
    };
    characters.sort_by_key(|name: &String| name.to_lowercase());

    Ok(LoginResult {
        ticket: object.get("ticket").and_then(value_text).unwrap_or_default(),
        characters,
        default_character: object.get("default_character").and_then(value_text).unwrap_or_default(),
        bookmarks: bookmark_names(object.get("bookmarks")),
        friends: friend_pairs(object.get("friends")),
    })
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn bookmark_names(items: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Value::Object(map) => map.get("name").and_then(value_text),
            other => value_text(other),
        })
        .filter(|name| !name.is_empty())
        .collect()
}

fn friend_pairs(items: Option<&Value>) -> Option<Vec<(String, String)>> {
    let Some(Value::Array(items)) = items else {
        return None;
    };
    let pairs = items
        .iter()
        .filter_map(Value::as_object)
        .map(|item| {
            let name = |key: &str| item.get(key).and_then(value_text).unwrap_or_default();
            (name("source_name"), name("dest_name"))
        })
        .collect();
    Some(pairs)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_characters_given_as_object() {
        let payload = json!({
            "ticket": "fct_abc",
            "default_character": "Zed",
            "characters": {"zed": 1, "Alice": 2},
            "bookmarks": [{"name": "Bob"}, "Carl"],
            "friends": [{"source_name": "Alice", "dest_name": "Dora"}],
        });
        let result = parse_login(&payload).unwrap();
        assert_eq!(result.ticket, "fct_abc");
        assert_eq!(result.characters, vec!["Alice", "zed"]);
        assert_eq!(result.bookmarks, vec!["Bob", "Carl"]);
        assert_eq!(result.friends, Some(vec![("Alice".to_string(), "Dora".to_string())]));
    }

    #[test]
    fn parses_characters_given_as_list() {
        let payload = json!({"ticket": "t", "characters": ["b", "A"], "friends": {}});
        let result = parse_login(&payload).unwrap();
        assert_eq!(result.characters, vec!["A", "b"]);
        assert_eq!(result.friends, None);
    }

    #[test]
    fn server_error_is_a_rejection() {
        let error = parse_login(&json!({"error": "Login failed"})).unwrap_err();
        assert!(!error.is_temporary());
        assert_eq!(error.to_string(), "Login failed");
    }

    #[test]
    fn non_object_answer_is_temporary() {
        assert!(parse_login(&json!([1, 2])).unwrap_err().is_temporary());
    }

    #[tokio::test]
    async fn missing_credentials_are_rejected() {
        let error = fetch_ticket("", "secret").await.unwrap_err();
        assert!(!error.is_temporary());
    }
}
