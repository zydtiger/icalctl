use super::*;

pub(super) const REDACTED: &str = "********";

#[derive(Clone, Copy)]
pub(super) enum KeyKind {
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

pub(super) fn report(
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

pub(super) fn init_file(path: &Path, allow_existing: bool) -> Result<()> {
    if path.exists() {
        if allow_existing {
            return Ok(());
        }
        bail!("configuration already exists: {}", path.display());
    }
    write_secure(path, SAMPLE_CONFIG.as_bytes())
}

pub(super) fn require_file(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "configuration does not exist: {}; run `icalctl config init`",
            path.display()
        );
    }
    Ok(())
}

pub(super) fn edit_file(path: &Path) -> Result<()> {
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

pub(super) fn require_known_key(key: &str) -> Result<KeyKind> {
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

pub(super) fn read_set_value(
    key: &str,
    kind: KeyKind,
    value: Option<String>,
    stdin: bool,
) -> Result<String> {
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

pub(super) fn set_value(path: &Path, key: &str, kind: KeyKind, input: &str) -> Result<()> {
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

pub(super) fn set_document_value(
    document: &mut DocumentMut,
    key: &str,
    new_value: Value,
) -> Result<()> {
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

pub(super) fn unset_value(path: &Path, key: &str) -> Result<()> {
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

pub(super) fn effective_value(config: &Config, key: &str) -> Result<String> {
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
