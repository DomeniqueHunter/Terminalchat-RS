//! Character profiles from the f-list.net JSON API.

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::{Map, Value};

use crate::error::{Error, Result};

pub const API_BASE: &str = "https://www.f-list.net/json/api/";
const TIMEOUT: Duration = Duration::from_secs(20);
const MAPPING_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
pub const KINK_CHOICES: [&str; 4] = ["fave", "yes", "maybe", "no"];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Profile {
    pub name: String,
    pub title: String,
    pub description: String,
    /// Groups of (label, value) pairs.
    pub infotags: Vec<(String, Vec<(String, String)>)>,
    /// Kink names per choice (`fave`, `yes`, `maybe`, `no`).
    pub kinks: Vec<(String, Vec<String>)>,
    /// Pairs of (file name, description).
    pub images: Vec<(String, String)>,
    pub views: i64,
    pub created: i64,
    pub updated: i64,
}

fn as_map(items: Option<&Value>) -> Map<String, Value> {
    match items {
        Some(Value::Object(map)) => map.clone(),
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?;
                let key = id.as_str().map(String::from).unwrap_or_else(|| id.to_string());
                Some((key, item.clone()))
            })
            .collect(),
        _ => Map::new(),
    }
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn number(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Value::String(text)) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn names_by_id(mapping: &Value, key: &str, field: &str) -> Map<String, Value> {
    as_map(mapping.get(key))
        .into_iter()
        .map(|(id, item)| (id, Value::String(text(item.get(field)))))
        .collect()
}

/// Combines the answer of `character-data.php` with the id mapping of `mapping-list.php`.
pub fn build_profile(data: &Value, mapping: &Value) -> Profile {
    let infotag_defs = as_map(mapping.get("infotags"));
    let group_names = names_by_id(mapping, "infotag_groups", "name");
    let list_items = names_by_id(mapping, "listitems", "value");
    let kink_names = names_by_id(mapping, "kinks", "name");
    let lookup = |map: &Map<String, Value>, id: &str| map.get(id).and_then(Value::as_str).map(String::from);

    let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
    if let Some(Value::Object(tags)) = data.get("infotags") {
        for (tag_id, value) in tags {
            let definition = infotag_defs.get(tag_id).cloned().unwrap_or(Value::Null);
            let name = match text(definition.get("name")) {
                name if name.is_empty() => format!("#{tag_id}"),
                name => name,
            };
            let mut value_text = text(Some(value));
            if definition.get("type").and_then(Value::as_str) == Some("list") {
                value_text = lookup(&list_items, &value_text).unwrap_or(value_text);
            }
            let group_id = text(definition.get("group_id"));
            let group = lookup(&group_names, &group_id).unwrap_or_else(|| "Other".into());
            match groups.iter_mut().find(|(title, _)| *title == group) {
                Some((_, entries)) => entries.push((name, value_text)),
                None => groups.push((group, vec![(name, value_text)])),
            }
        }
    }
    for (_, entries) in &mut groups {
        entries.sort();
    }

    let mut kinks: Vec<(String, Vec<String>)> = KINK_CHOICES.iter().map(|c| (c.to_string(), Vec::new())).collect();
    let mut add_kink = |choice: &str, name: String| {
        if let Some((_, names)) = kinks.iter_mut().find(|(key, _)| key == choice) {
            names.push(name);
        }
    };
    if let Some(Value::Object(raw)) = data.get("kinks") {
        for (kink_id, choice) in raw {
            let name = lookup(&kink_names, kink_id).unwrap_or_else(|| format!("#{kink_id}"));
            add_kink(choice.as_str().unwrap_or(""), name);
        }
    }
    if let Some(Value::Object(custom)) = data.get("custom_kinks") {
        for item in custom.values() {
            let name = format!("{} (custom)", text(item.get("name")).trim());
            add_kink(item.get("choice").and_then(Value::as_str).unwrap_or(""), name);
        }
    }
    for (_, names) in &mut kinks {
        names.sort_by_key(|name| name.to_lowercase());
    }

    let raw_images: Vec<Value> = match data.get("images") {
        Some(Value::Array(list)) => list.clone(),
        Some(Value::Object(map)) => map.values().cloned().collect(),
        _ => Vec::new(),
    };
    let mut raw_images: Vec<&Value> = raw_images.iter().filter(|item| item.is_object()).collect();
    raw_images.sort_by_key(|item| number(item.get("sort_order")));
    let images = raw_images
        .into_iter()
        .filter_map(|item| {
            let image_id = match text(item.get("image_id")) {
                id if id.is_empty() => text(item.get("id")),
                id => id,
            };
            let extension = text(item.get("extension"));
            let valid = !image_id.is_empty()
                && image_id.chars().all(|c| c.is_ascii_digit())
                && !extension.is_empty()
                && extension.chars().all(|c| c.is_ascii_alphanumeric());
            valid.then(|| (format!("{image_id}.{extension}"), text(item.get("description"))))
        })
        .collect();

    Profile {
        name: text(data.get("name")),
        title: text(data.get("custom_title")),
        description: text(data.get("description")),
        infotags: groups,
        kinks,
        images,
        views: number(data.get("views")),
        created: number(data.get("created_at")),
        updated: number(data.get("updated_at")),
    }
}

async fn request(endpoint: &str, form: &[(&str, &str)]) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| Error::Temporary(format!("Request failed: {error}")))?;
    let response = client
        .post(format!("{API_BASE}{endpoint}"))
        .form(form)
        .send()
        .await
        .map_err(|error| Error::Temporary(format!("Request failed: {error}")))?;
    if !response.status().is_success() {
        return Err(Error::Temporary(format!("f-list answered with HTTP {}", response.status().as_u16())));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|_| Error::Temporary("Unexpected answer from f-list".into()))?;
    if !payload.is_object() {
        return Err(Error::Temporary("Unexpected answer from f-list".into()));
    }
    if let Some(message) = payload.get("error").and_then(Value::as_str).filter(|text| !text.is_empty()) {
        return Err(Error::Rejected(message.to_string()));
    }
    Ok(payload)
}

/// Returns the id mapping, from the cache file if it is younger than a week.
pub async fn mapping(cache_file: &Path) -> Result<Value> {
    let fresh = std::fs::metadata(cache_file)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < MAPPING_MAX_AGE);
    if fresh {
        if let Some(value) = std::fs::read_to_string(cache_file).ok().and_then(|text| serde_json::from_str(&text).ok()) {
            return Ok(value);
        }
    }
    let value = request("mapping-list.php", &[]).await?;
    if let Some(parent) = cache_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(cache_file, value.to_string());
    Ok(value)
}

/// Loads a profile. An error mentioning the ticket means the ticket expired.
pub async fn fetch_profile(account: &str, ticket: &str, name: &str, mapping_file: &Path) -> Result<Profile> {
    let data = request("character-data.php", &[("account", account), ("ticket", ticket), ("name", name)]).await?;
    let mapping = mapping(mapping_file).await?;
    Ok(build_profile(&data, &mapping))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn builds_profile_from_ids() {
        let mapping = json!({
            "infotags": [
                {"id": "1", "name": "Gender", "type": "list", "group_id": "3"},
                {"id": "2", "name": "Age", "type": "text", "group_id": "1"}
            ],
            "infotag_groups": [{"id": "1", "name": "General details"}, {"id": "3", "name": "Character"}],
            "listitems": [{"id": "9", "name": "gender", "value": "Female"}],
            "kinks": [{"id": "5", "name": "Cuddling"}, {"id": "6", "name": "Rain"}],
        });
        let data = json!({
            "name": "Alice", "custom_title": "Explorer", "description": "[b]Hi[/b]",
            "infotags": {"1": "9", "2": "25"},
            "kinks": {"5": "fave", "6": "no"},
            "custom_kinks": {"1": {"name": "Tea", "choice": "yes"}},
            "images": [{"image_id": "12", "extension": "png", "description": "me", "sort_order": "1"}],
        });
        let profile = build_profile(&data, &mapping);
        assert_eq!(profile.name, "Alice");
        assert_eq!(profile.infotags.len(), 2);
        assert!(profile.infotags.iter().any(|(group, entries)| group == "Character" && entries[0].1 == "Female"));
        assert_eq!(profile.kinks[0], ("fave".to_string(), vec!["Cuddling".to_string()]));
        assert_eq!(profile.kinks[1].1, vec!["Tea (custom)".to_string()]);
        assert_eq!(profile.images, vec![("12.png".to_string(), "me".to_string())]);
    }
}
