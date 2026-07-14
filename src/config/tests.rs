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
