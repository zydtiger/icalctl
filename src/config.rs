use crate::cli::ConfigCommand;
use crate::models::{ConfigReport, JsonOutput};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use toml_edit::{DocumentMut, Item, Value, value};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

const CONFIG_VERSION: u8 = 1;
pub const MAX_TRAVEL_RANGE_DAYS: u32 = 366;
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const REDACTED: &str = "********";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static TEST_CONFIG_PATH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

const SAMPLE_CONFIG: &str = r#"# icalctl configuration
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
default_range_days = 90
calendar_ids = []

[travel.server]
bind = "127.0.0.1"
port = 0
open_browser = true

[travel.map]
projection = "globe"
# style_url = "https://example.com/map-style.json"
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

#[derive(Clone, Copy)]
enum KeyKind {
    Secret,
    String,
    Bool,
    Integer,
    StringArray,
}

const CONFIG_KEYS: &[(&str, KeyKind)] = &[
    ("flightaware.api_key", KeyKind::Secret),
    ("flightaware.enabled", KeyKind::Bool),
    ("flightaware.monthly_result_set_limit", KeyKind::Integer),
    ("flightaware.request_timeout_seconds", KeyKind::Integer),
    ("flightaware.stale_if_error", KeyKind::Bool),
    ("calendar.default_calendar_id", KeyKind::String),
    ("reminders.default_list_id", KeyKind::String),
    ("travel.default_range_days", KeyKind::Integer),
    ("travel.calendar_ids", KeyKind::StringArray),
    ("travel.server.bind", KeyKind::String),
    ("travel.server.port", KeyKind::Integer),
    ("travel.server.open_browser", KeyKind::Bool),
    ("travel.map.projection", KeyKind::String),
    ("travel.map.style_url", KeyKind::String),
];

pub fn run(command: ConfigCommand) -> Result<JsonOutput> {
    let path = config_path()?;
    match command {
        ConfigCommand::Path => Ok(report("path", &path, None, None, None)),
        ConfigCommand::Init => {
            init_file(&path, false)?;
            Ok(report("initialized", &path, None, None, None))
        }
        ConfigCommand::Show => {
            require_file(&path)?;
            let mut config = load_from(&path)?;
            if config.flightaware.api_key.is_some() {
                config.flightaware.api_key = Some(REDACTED.to_string());
            }
            let contents = toml::to_string_pretty(&config)
                .context("failed to serialize effective configuration")?;
            Ok(report("show", &path, None, None, Some(contents)))
        }
        ConfigCommand::Validate => {
            require_file(&path)?;
            load_from(&path)?;
            Ok(report("valid", &path, None, None, None))
        }
        ConfigCommand::Edit => {
            init_file(&path, true)?;
            edit_file(&path)?;
            Ok(report("edited", &path, None, None, None))
        }
        ConfigCommand::Get { key } => {
            require_known_key(&key)?;
            let config = load()?;
            let value = effective_value(&config, &key)?;
            Ok(report("get", &path, Some(key), Some(value), None))
        }
        ConfigCommand::Set { key, value, stdin } => {
            let kind = require_known_key(&key)?;
            init_file(&path, true)?;
            load_from(&path)?;
            let input = read_set_value(&key, kind, value, stdin)?;
            set_value(&path, &key, kind, &input)?;
            let shown = if matches!(kind, KeyKind::Secret) {
                REDACTED.to_string()
            } else {
                effective_value(&load_from(&path)?, &key)?
            };
            Ok(report("set", &path, Some(key), Some(shown), None))
        }
        ConfigCommand::Unset { key } => {
            require_known_key(&key)?;
            require_file(&path)?;
            unset_value(&path, &key)?;
            Ok(report("unset", &path, Some(key), None, None))
        }
    }
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    load_from(&path)
}

pub fn default_calendar_id() -> Result<Option<String>> {
    Ok(load()?.calendar.default_calendar_id)
}

pub fn default_reminder_list_id() -> Result<Option<String>> {
    Ok(load()?.reminders.default_list_id)
}

#[cfg(test)]
pub(crate) fn with_test_config<T>(path: &Path, operation: impl FnOnce() -> T) -> T {
    TEST_CONFIG_PATH.with(|slot| {
        let previous = slot.replace(Some(path.to_path_buf()));
        let result = operation();
        slot.replace(previous);
        result
    })
}

#[cfg(test)]
pub(crate) fn with_test_config_contents<T>(
    name: &str,
    contents: &str,
    operation: impl FnOnce() -> T,
) -> T {
    let path = std::env::temp_dir()
        .join(format!(
            "icalctl-config-integration-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
        .join("config.toml");
    write_secure(&path, contents.as_bytes()).unwrap();
    let result = with_test_config(&path, operation);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
    result
}

fn config_path() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_CONFIG_PATH.with(|path| path.borrow().clone()) {
        return Ok(path);
    }
    #[cfg(test)]
    return Ok(std::env::temp_dir()
        .join("icalctl-test-default-config")
        .join(std::thread::current().name().unwrap_or("test"))
        .join("config.toml"));

    #[cfg(not(test))]
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    #[cfg(not(test))]
    Ok(PathBuf::from(home).join(".icalctl").join("config.toml"))
}

fn load_from(path: &Path) -> Result<Config> {
    let contents = read_secure_string(path)?;
    let config: Config = toml::from_str(&contents).map_err(|error| {
        let location = error
            .span()
            .map(|span| line_and_column(&contents, span.start));
        match location {
            Some((line, column)) => anyhow!(
                "failed to parse configuration {} at line {line}, column {column}",
                path.display()
            ),
            None => anyhow!("failed to parse configuration {}", path.display()),
        }
    })?;
    validate(&config)?;
    Ok(config)
}

#[cfg(unix)]
fn read_secure(path: &Path) -> Result<Vec<u8>> {
    let parent = path.parent().context("configuration path has no parent")?;
    validate_existing_directory(parent)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open configuration {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect configuration {}", path.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", path.display());
    }
    let expected_uid = unsafe { libc::geteuid() };
    if metadata.uid() != expected_uid {
        bail!(
            "configuration is not owned by the current user: {}",
            path.display()
        );
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!(
            "configuration permissions are too broad: {}; run `chmod 600 {}`",
            path.display(),
            path.display()
        );
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        bail!(
            "configuration is too large: {} exceeds {} bytes",
            path.display(),
            MAX_CONFIG_BYTES
        );
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut contents)
        .with_context(|| format!("failed to read configuration {}", path.display()))?;
    Ok(contents)
}

#[cfg(not(unix))]
fn read_secure(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect configuration {}", path.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        bail!(
            "configuration is too large: {} exceeds {} bytes",
            path.display(),
            MAX_CONFIG_BYTES
        );
    }
    fs::read(path).with_context(|| format!("failed to read configuration {}", path.display()))
}

fn read_secure_string(path: &Path) -> Result<String> {
    String::from_utf8(read_secure(path)?)
        .with_context(|| format!("configuration is not valid UTF-8: {}", path.display()))
}

#[cfg(unix)]
fn validate_existing_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "configuration directory must be a real directory: {}",
            path.display()
        );
    }
    let expected_uid = unsafe { libc::geteuid() };
    if metadata.uid() != expected_uid {
        bail!(
            "configuration directory is not owned by the current user: {}",
            path.display()
        );
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!(
            "configuration directory permissions are too broad: {}; run `chmod 700 {}`",
            path.display(),
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_existing_directory(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!(
            "configuration directory is not a directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn line_and_column(contents: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = &contents[..byte_offset.min(contents.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, tail)| {
            tail.chars().count() + 1
        });
    (line, column)
}

fn validate(config: &Config) -> Result<()> {
    if config.config_version != CONFIG_VERSION {
        bail!(
            "unsupported config_version {}; expected {CONFIG_VERSION}",
            config.config_version
        );
    }
    if config
        .flightaware
        .api_key
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("flightaware.api_key must not be empty");
    }
    if config.flightaware.monthly_result_set_limit == 0 {
        bail!("flightaware.monthly_result_set_limit must be greater than zero");
    }
    if config.flightaware.request_timeout_seconds == 0 {
        bail!("flightaware.request_timeout_seconds must be greater than zero");
    }
    if config
        .calendar
        .default_calendar_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("calendar.default_calendar_id must not be empty");
    }
    if config
        .reminders
        .default_list_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("reminders.default_list_id must not be empty");
    }
    if config.travel.default_range_days == 0 {
        bail!("travel.default_range_days must be greater than zero");
    }
    if config.travel.default_range_days > MAX_TRAVEL_RANGE_DAYS {
        bail!("travel.default_range_days must not exceed {MAX_TRAVEL_RANGE_DAYS} inclusive days");
    }
    if config
        .travel
        .calendar_ids
        .iter()
        .any(|id| id.trim().is_empty())
    {
        bail!("travel.calendar_ids must not contain empty values");
    }
    let bind: IpAddr = config
        .travel
        .server
        .bind
        .parse()
        .context("travel.server.bind must be an IP address")?;
    if !bind.is_loopback() {
        bail!("travel.server.bind must be a loopback address");
    }
    if !matches!(config.travel.map.projection.as_str(), "map" | "globe") {
        bail!("travel.map.projection must be `map` or `globe`");
    }
    if config
        .travel
        .map
        .style_url
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("travel.map.style_url must not be empty");
    }
    Ok(())
}

fn report(
    action: &str,
    path: &Path,
    key: Option<String>,
    value: Option<String>,
    contents: Option<String>,
) -> JsonOutput {
    JsonOutput::Config {
        config: ConfigReport {
            action: action.to_string(),
            path: path.display().to_string(),
            key,
            value,
            contents,
        },
    }
}

fn init_file(path: &Path, allow_existing: bool) -> Result<()> {
    if path.exists() {
        if allow_existing {
            return Ok(());
        }
        bail!("configuration already exists: {}", path.display());
    }
    write_secure(path, SAMPLE_CONFIG.as_bytes())
}

fn require_file(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "configuration does not exist: {}; run `icalctl config init`",
            path.display()
        );
    }
    Ok(())
}

fn write_secure(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("configuration path has no parent")?;
    if parent.exists() {
        validate_existing_directory(parent)?;
    } else {
        fs::create_dir(parent).with_context(|| format!("failed to create {}", parent.display()))?;
        #[cfg(unix)]
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", parent.display()))?;
        validate_existing_directory(parent)?;
    }

    for _ in 0..100 {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".config.toml.{}.{}.tmp",
            std::process::id(),
            suffix
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create {}", temporary.display()));
            }
        };
        let write_result = (|| -> Result<()> {
            file.write_all(contents)
                .with_context(|| format!("failed to write {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync {}", temporary.display()))?;
            drop(file);
            fs::rename(&temporary, path)
                .with_context(|| format!("failed to replace {}", path.display()))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return write_result;
    }
    bail!("failed to create a unique temporary configuration file")
}

fn edit_file(path: &Path) -> Result<()> {
    load_from(path)?;
    let original = read_secure(path)?;
    let status = Command::new("/bin/sh")
        .args([
            "-c",
            "exec ${VISUAL:-${EDITOR:-/usr/bin/vi}} \"$ICALCTL_CONFIG_PATH\"",
        ])
        .env("ICALCTL_CONFIG_PATH", path)
        .status()
        .context("failed to launch configuration editor")?;
    if !status.success() {
        write_secure(path, &original)?;
        bail!("configuration editor exited with {status}; previous configuration restored");
    }
    if let Err(error) = load_from(path) {
        write_secure(path, &original)?;
        bail!("edited configuration is invalid: {error:#}; previous configuration restored");
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", path.display()))?;
    Ok(())
}

fn require_known_key(key: &str) -> Result<KeyKind> {
    CONFIG_KEYS
        .iter()
        .find_map(|(candidate, kind)| (*candidate == key).then_some(*kind))
        .ok_or_else(|| {
            anyhow!(
                "unknown configuration key {key:?}; valid keys: {}",
                CONFIG_KEYS
                    .iter()
                    .map(|(key, _)| *key)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn read_set_value(key: &str, kind: KeyKind, value: Option<String>, stdin: bool) -> Result<String> {
    if matches!(kind, KeyKind::Secret) && value.is_some() {
        bail!("{key} must be entered through the hidden prompt or --stdin, not as VALUE");
    }
    if let Some(value) = value {
        return Ok(value);
    }
    if stdin {
        let mut value = String::new();
        io::stdin()
            .read_to_string(&mut value)
            .context("failed to read configuration value from stdin")?;
        return Ok(value.trim_end_matches(['\r', '\n']).to_string());
    }
    if !io::stdin().is_terminal() {
        bail!("VALUE is required when stdin is not a terminal; pass VALUE or --stdin");
    }
    if matches!(kind, KeyKind::Secret) {
        return rpassword::prompt_password(format!("{key}: "))
            .context("failed to read secret configuration value");
    }
    print!("{key}: ");
    io::stdout().flush().context("failed to flush prompt")?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .context("failed to read configuration value")?;
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
}

fn set_value(path: &Path, key: &str, kind: KeyKind, input: &str) -> Result<()> {
    if input.is_empty() {
        bail!("configuration value must not be empty; use `config unset` to remove it");
    }
    let contents = read_secure_string(path)?;
    let mut document = contents.parse::<DocumentMut>().map_err(|_| {
        anyhow!(
            "failed to parse configuration {}; run `icalctl config edit`",
            path.display()
        )
    })?;
    let new_value = match kind {
        KeyKind::Secret | KeyKind::String => Value::from(input),
        KeyKind::Bool => Value::from(
            input
                .parse::<bool>()
                .with_context(|| format!("{key} must be true or false"))?,
        ),
        KeyKind::Integer => Value::from(
            input
                .parse::<i64>()
                .with_context(|| format!("{key} must be an integer"))?,
        ),
        KeyKind::StringArray => {
            let parsed = input
                .parse::<Value>()
                .with_context(|| format!("{key} must be a TOML string array"))?;
            if !parsed.is_array()
                || parsed
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| !value.is_str()))
            {
                bail!("{key} must be a TOML string array");
            }
            parsed
        }
    };
    set_document_value(&mut document, key, new_value)?;
    let rendered = document.to_string();
    let parsed: Config = toml::from_str(&rendered).context("updated configuration is invalid")?;
    validate(&parsed)?;
    write_secure(path, rendered.as_bytes())
}

fn set_document_value(document: &mut DocumentMut, key: &str, new_value: Value) -> Result<()> {
    let parts = key.split('.').collect::<Vec<_>>();
    let (name, parents) = parts.split_last().context("configuration key is empty")?;
    let mut table = document.as_table_mut();
    for parent in parents {
        if !table.contains_key(parent) {
            table.insert(parent, Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(parent)
            .and_then(Item::as_table_mut)
            .with_context(|| format!("configuration path {} is not a table", parent))?;
    }
    table.insert(name, value(new_value));
    Ok(())
}

fn unset_value(path: &Path, key: &str) -> Result<()> {
    load_from(path)?;
    let contents = read_secure_string(path)?;
    let mut document = contents.parse::<DocumentMut>().map_err(|_| {
        anyhow!(
            "failed to parse configuration {}; run `icalctl config edit`",
            path.display()
        )
    })?;
    let parts = key.split('.').collect::<Vec<_>>();
    let (name, parents) = parts.split_last().context("configuration key is empty")?;
    let mut table = document.as_table_mut();
    for parent in parents {
        let Some(next) = table.get_mut(parent).and_then(Item::as_table_mut) else {
            bail!("configuration key is not set: {key}");
        };
        table = next;
    }
    if table.remove(name).is_none() {
        bail!("configuration key is not set: {key}");
    }
    let rendered = document.to_string();
    let parsed: Config = toml::from_str(&rendered).context("updated configuration is invalid")?;
    validate(&parsed)?;
    write_secure(path, rendered.as_bytes())
}

fn effective_value(config: &Config, key: &str) -> Result<String> {
    if key == "flightaware.api_key" {
        return Ok(if config.flightaware.api_key.is_some() {
            REDACTED.to_string()
        } else {
            "<unset>".to_string()
        });
    }
    let optional_value = match key {
        "calendar.default_calendar_id" => config.calendar.default_calendar_id.as_deref(),
        "reminders.default_list_id" => config.reminders.default_list_id.as_deref(),
        "travel.map.style_url" => config.travel.map.style_url.as_deref(),
        _ => None,
    };
    if matches!(
        key,
        "calendar.default_calendar_id" | "reminders.default_list_id" | "travel.map.style_url"
    ) {
        return Ok(optional_value.unwrap_or("<unset>").to_string());
    }
    let serialized = toml::to_string(config).context("failed to serialize configuration")?;
    let value: toml::Value = toml::from_str(&serialized)?;
    let mut current = &value;
    for part in key.split('.') {
        current = current
            .get(part)
            .with_context(|| format!("configuration key is not set: {key}"))?;
    }
    Ok(match current {
        toml::Value::String(value) => value.clone(),
        value => value.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "icalctl-config-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::create_dir_all(&directory).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        directory.join("config.toml")
    }

    #[test]
    fn sample_is_valid_and_has_safe_defaults() {
        let config: Config = toml::from_str(SAMPLE_CONFIG).unwrap();
        validate(&config).unwrap();
        assert!(config.flightaware.api_key.is_none());
        assert_eq!(config.flightaware.monthly_result_set_limit, 900);
        assert_eq!(config.travel.server.bind, "127.0.0.1");
    }

    #[test]
    fn set_preserves_comments_and_validates_types() {
        let path = temp_config("set");
        write_secure(&path, SAMPLE_CONFIG.as_bytes()).unwrap();
        set_value(
            &path,
            "calendar.default_calendar_id",
            KeyKind::String,
            "CAL-1",
        )
        .unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("# Exact EventKit calendar ID"));
        assert_eq!(
            load_from(&path)
                .unwrap()
                .calendar
                .default_calendar_id
                .as_deref(),
            Some("CAL-1")
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unset_restores_the_effective_default() {
        let path = temp_config("unset");
        write_secure(&path, SAMPLE_CONFIG.as_bytes()).unwrap();
        set_value(&path, "travel.server.port", KeyKind::Integer, "8123").unwrap();
        unset_value(&path, "travel.server.port").unwrap();
        assert_eq!(load_from(&path).unwrap().travel.server.port, 0);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn api_key_is_redacted() {
        let mut config = Config::default();
        config.flightaware.api_key = Some("secret".to_string());
        assert_eq!(
            effective_value(&config, "flightaware.api_key").unwrap(),
            REDACTED
        );
    }

    #[test]
    fn optional_values_report_unset_instead_of_failing() {
        let config = Config::default();
        assert_eq!(
            effective_value(&config, "calendar.default_calendar_id").unwrap(),
            "<unset>"
        );
        assert_eq!(
            effective_value(&config, "reminders.default_list_id").unwrap(),
            "<unset>"
        );
    }

    #[test]
    fn secret_values_reject_positional_arguments() {
        let error = read_set_value(
            "flightaware.api_key",
            KeyKind::Secret,
            Some("secret".to_string()),
            false,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("hidden prompt or --stdin"));
        assert!(!error.contains("secret"));
    }

    #[cfg(unix)]
    #[test]
    fn insecure_existing_permissions_are_rejected() {
        let path = temp_config("permissions");
        write_secure(&path, SAMPLE_CONFIG.as_bytes()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        let error = load_from(&path).unwrap_err().to_string();

        assert!(error.contains("chmod 600"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_config_directory_is_rejected_before_writing() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "icalctl-config-symlink-dir-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let target = root.join("target");
        let linked = root.join("linked");
        fs::create_dir_all(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&target, &linked).unwrap();

        let error = write_secure(&linked.join("config.toml"), SAMPLE_CONFIG.as_bytes())
            .unwrap_err()
            .to_string();

        assert!(error.contains("real directory"));
        assert!(!target.join("config.toml").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_config_file_is_rejected_before_reading() {
        use std::os::unix::fs::symlink;

        let path = temp_config("symlink-file");
        let parent = path.parent().unwrap();
        let target = parent.join("target.toml");
        write_secure(&target, SAMPLE_CONFIG.as_bytes()).unwrap();
        symlink(&target, &path).unwrap();

        let error = load_from(&path).unwrap_err().to_string();

        assert!(error.contains("failed to open configuration"));
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn non_loopback_server_bind_is_rejected() {
        let mut config = Config::default();
        config.travel.server.bind = "0.0.0.0".to_string();
        assert!(validate(&config).is_err());
    }

    #[test]
    fn oversized_default_travel_range_is_rejected() {
        let mut config = Config::default();
        config.travel.default_range_days = MAX_TRAVEL_RANGE_DAYS + 1;
        assert!(validate(&config).is_err());
    }

    #[test]
    fn parse_errors_do_not_echo_secret_source_lines() {
        let path = temp_config("parse-secret");
        write_secure(
            &path,
            b"config_version = 1\n[flightaware]\napi_key = \"super-secret\n",
        )
        .unwrap();

        let error = load_from(&path).unwrap_err().to_string();

        assert!(error.contains("line 3"));
        assert!(!error.contains("super-secret"));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
