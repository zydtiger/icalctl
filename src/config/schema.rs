use super::*;

pub(super) const CONFIG_VERSION: u8 = 1;
pub const MAX_TRAVEL_RANGE_DAYS: u32 = 366;

pub(super) const SAMPLE_CONFIG: &str = r#"# icalctl configuration
# This file may contain secrets. Keep its permissions set to 0600.

config_version = 1

[flightaware]
# api_key = "replace-with-your-flightaware-api-key"
enabled = true
monthly_result_set_limit = 900
request_timeout_seconds = 10
stale_if_error = true

[calendar]
# Exact EventKit calendar ID used for adds that omit a calendar selector.
# default_calendar_id = "CALENDAR-ID"

[reminders]
# Exact EventKit reminder-list ID used for adds that omit a list selector.
# default_list_id = "REMINDER-LIST-ID"

[travel]
# Inclusive upcoming window used by travel serve (maximum 366 days).
default_range_days = 90
calendar_ids = []

[travel.server]
bind = "127.0.0.1"
port = 0
open_browser = true

[travel.map]
projection = "globe"
# Optional public/keyless MapLibre style URL.
# Defaults to the OpenFreeMap Bright street style.
# style_url = "https://tiles.openfreemap.org/styles/bright"
"#;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub config_version: u8,
    pub flightaware: FlightAwareConfig,
    pub calendar: CalendarConfig,
    pub reminders: RemindersConfig,
    pub travel: TravelConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            flightaware: FlightAwareConfig::default(),
            calendar: CalendarConfig::default(),
            reminders: RemindersConfig::default(),
            travel: TravelConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlightAwareConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub enabled: bool,
    pub monthly_result_set_limit: u32,
    pub request_timeout_seconds: u64,
    pub stale_if_error: bool,
}

impl Default for FlightAwareConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            enabled: true,
            monthly_result_set_limit: 900,
            request_timeout_seconds: 10,
            stale_if_error: true,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct CalendarConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_calendar_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemindersConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_list_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TravelConfig {
    pub default_range_days: u32,
    pub calendar_ids: Vec<String>,
    pub server: TravelServerConfig,
    pub map: TravelMapConfig,
}

impl Default for TravelConfig {
    fn default() -> Self {
        Self {
            default_range_days: 90,
            calendar_ids: Vec::new(),
            server: TravelServerConfig::default(),
            map: TravelMapConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TravelServerConfig {
    pub bind: String,
    pub port: u16,
    pub open_browser: bool,
}

impl Default for TravelServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 0,
            open_browser: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TravelMapConfig {
    pub projection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style_url: Option<String>,
}

impl Default for TravelMapConfig {
    fn default() -> Self {
        Self {
            projection: "globe".to_string(),
            style_url: None,
        }
    }
}
