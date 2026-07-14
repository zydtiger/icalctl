use super::*;

pub(super) fn validate(config: &Config) -> Result<()> {
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
