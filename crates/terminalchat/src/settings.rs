//! The general settings of the settings page: every entry of config.json except account, password
//! and character. Each entry says how it is edited, how it is checked and when it takes effect, so
//! a new setting needs one line in `SETTINGS` and one in `value` and `apply`.

use crate::config::Config;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Text,
    Bool,
    Number { min: u32, max: u32 },
    Decimal,
    /// Comma separated.
    List,
    Choice(&'static [&'static str]),
    /// "auto" or width x height in pixels, e.g. "8x18".
    CellSize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    Now,
    Reconnect,
    Login,
    Restart,
}

impl Effect {
    pub fn note(self) -> &'static str {
        match self {
            Effect::Now => "",
            Effect::Reconnect => "after /reconnect",
            Effect::Login => "at next login",
            Effect::Restart => "after restart",
        }
    }
}

pub struct Setting {
    pub group: &'static str,
    pub key: &'static str,
    pub field: Field,
    pub effect: Effect,
}

pub const SETTINGS: [Setting; 14] = [
    Setting { group: "Connection", key: "server", field: Field::Text, effect: Effect::Reconnect },
    Setting { group: "Connection", key: "protocol", field: Field::Choice(&["wss://", "ws://"]), effect: Effect::Reconnect },
    Setting { group: "Connection", key: "endpoint", field: Field::Text, effect: Effect::Reconnect },
    Setting { group: "Start", key: "default_channels", field: Field::List, effect: Effect::Login },
    Setting { group: "Start", key: "remember_password", field: Field::Bool, effect: Effect::Now },
    Setting { group: "Display", key: "show_sidebar", field: Field::Bool, effect: Effect::Now },
    Setting { group: "Display", key: "show_ads", field: Field::Bool, effect: Effect::Now },
    Setting { group: "Images", key: "eicons", field: Field::Choice(&["inline", "off"]), effect: Effect::Restart },
    Setting { group: "Images", key: "eicon_height", field: Field::Number { min: 1, max: 20 }, effect: Effect::Now },
    Setting { group: "Images", key: "eicon_animate", field: Field::Bool, effect: Effect::Now },
    Setting { group: "Images", key: "eicon_max_age_hours", field: Field::Decimal, effect: Effect::Now },
    Setting { group: "Images", key: "cell_size", field: Field::CellSize, effect: Effect::Restart },
    Setting { group: "Logs", key: "log_keep_days", field: Field::Number { min: 0, max: 36500 }, effect: Effect::Now },
    Setting { group: "Data", key: "data_dir", field: Field::Text, effect: Effect::Restart },
];


pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|setting| setting.key == key)
}


/// The value as shown and edited on the settings page.
pub fn value(config: &Config, key: &str) -> String {
    match key {
        "server" => config.server.clone(),
        "protocol" => config.protocol.clone(),
        "endpoint" => config.endpoint.clone(),
        "default_channels" => config.default_channels.join(", "),
        "remember_password" => config.remember_password.to_string(),
        "show_sidebar" => config.show_sidebar.to_string(),
        "show_ads" => config.show_ads.to_string(),
        "eicons" => config.eicons.clone(),
        "eicon_height" => config.eicon_height.to_string(),
        "eicon_animate" => config.eicon_animate.to_string(),
        "eicon_max_age_hours" => format_decimal(config.eicon_max_age_hours),
        "cell_size" => config.cell_size.clone().unwrap_or_else(|| "auto".into()),
        "log_keep_days" => config.log_keep_days.to_string(),
        "data_dir" => config.data_dir.clone().unwrap_or_default(),
        _ => String::new(),
    }
}


/// The value of a fresh config.
pub fn default_value(key: &str) -> String {
    value(&Config::default(), key)
}


/// Checks a value and writes it into the config. The caller saves the config and applies what
/// takes effect at once.
pub fn apply(config: &mut Config, key: &str, text: &str) -> Result<(), String> {
    let setting = setting(key).ok_or_else(|| format!("unknown setting {key}"))?;
    let text = text.trim();
    match setting.field {
        Field::Text if key != "data_dir" && text.is_empty() => return Err("must not be empty".into()),
        Field::Choice(choices) if !choices.contains(&text) => return Err(format!("one of {}", choices.join(", "))),
        _ => {}
    }
    match key {
        "server" => config.server = text.to_string(),
        "protocol" => config.protocol = text.to_string(),
        "endpoint" => config.endpoint = text.to_string(),
        "default_channels" => {
            config.default_channels = text.split(',').map(str::trim).filter(|name| !name.is_empty()).map(String::from).collect();
        }
        "remember_password" => config.remember_password = parse_bool(text)?,
        "show_sidebar" => config.show_sidebar = parse_bool(text)?,
        "show_ads" => config.show_ads = parse_bool(text)?,
        "eicons" => config.eicons = text.to_string(),
        "eicon_height" => config.eicon_height = parse_number(text, setting.field)?,
        "eicon_animate" => config.eicon_animate = parse_bool(text)?,
        "eicon_max_age_hours" => {
            let hours = text.replace(',', ".").parse::<f64>().map_err(|_| "a number of hours, e.g. 24 or 0.5")?;
            if !hours.is_finite() || hours < 0.0 {
                return Err("0 or more hours".into());
            }
            config.eicon_max_age_hours = hours;
        }
        "cell_size" => config.cell_size = parse_cell_size(text)?,
        "log_keep_days" => config.log_keep_days = parse_number(text, setting.field)?,
        "data_dir" => config.data_dir = (!text.is_empty()).then(|| text.to_string()),
        _ => return Err(format!("unknown setting {key}")),
    }
    Ok(())
}


fn parse_bool(text: &str) -> Result<bool, String> {
    match text.to_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err("on or off".into()),
    }
}


fn parse_number(text: &str, field: Field) -> Result<u32, String> {
    let Field::Number { min, max } = field else { return Err("not a number".into()) };
    match text.parse::<u32>() {
        Ok(number) if (min..=max).contains(&number) => Ok(number),
        _ => Err(format!("a whole number from {min} to {max}")),
    }
}


fn parse_cell_size(text: &str) -> Result<Option<String>, String> {
    let lower = text.to_lowercase();
    if lower.is_empty() || lower == "auto" {
        return Ok(None);
    }
    let valid = lower
        .split_once('x')
        .and_then(|(width, height)| Some((width.trim().parse::<u16>().ok()?, height.trim().parse::<u16>().ok()?)))
        .is_some_and(|(width, height)| width > 0 && height > 0);
    if valid { Ok(Some(lower.replace(' ', ""))) } else { Err("auto or width x height in pixels, e.g. 8x18".into()) }
}


fn format_decimal(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value:.0}") } else { value.to_string() }
}


#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn every_setting_reads_back_what_was_written() {
        let mut config = Config::default();
        let values = [
            ("server", "example.net"),
            ("protocol", "ws://"),
            ("endpoint", "/chat3"),
            ("default_channels", "Frontpage, Helpdesk"),
            ("remember_password", "false"),
            ("show_sidebar", "true"),
            ("show_ads", "false"),
            ("eicons", "off"),
            ("eicon_height", "6"),
            ("eicon_animate", "false"),
            ("eicon_max_age_hours", "0.5"),
            ("cell_size", "8x18"),
            ("log_keep_days", "30"),
            ("data_dir", "/tmp/terminalchat"),
        ];
        assert_eq!(values.len(), SETTINGS.len());
        for (key, text) in values {
            apply(&mut config, key, text).unwrap();
            assert_eq!(value(&config, key), text, "{key}");
        }
    }


    #[test]
    fn wrong_values_are_refused_and_change_nothing() {
        let mut config = Config::default();
        for (key, text) in [
            ("server", " "),
            ("protocol", "http://"),
            ("eicon_height", "0"),
            ("eicon_height", "21"),
            ("eicon_max_age_hours", "-1"),
            ("cell_size", "8 by 18"),
            ("log_keep_days", "a week"),
            ("remember_password", "maybe"),
            ("account", "someone"),
        ] {
            assert!(apply(&mut config, key, text).is_err(), "{key} = {text}");
        }
        assert_eq!(config, Config::default());
    }


    #[test]
    fn empty_values_mean_the_default_where_that_makes_sense() {
        let mut config = Config::default();
        apply(&mut config, "cell_size", "8x18").unwrap();
        apply(&mut config, "cell_size", "").unwrap();
        apply(&mut config, "data_dir", "").unwrap();
        apply(&mut config, "default_channels", " , ").unwrap();
        assert_eq!((config.cell_size, config.data_dir, config.default_channels), (None, None, Vec::<String>::new()));
        assert_eq!(default_value("eicon_max_age_hours"), "24");
    }
}
