use crate::config::FlightAwareConfig;
use crate::travel::{TravelAirport, TravelCollection, TravelLeg, TravelWarning};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::cmp::Ordering;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::Duration as StdDuration;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const PROVIDER: &str = "flightaware";
const BASE_URL: &str = "https://aeroapi.flightaware.com/aeroapi";
const CACHE_VERSION: u8 = 1;
const USAGE_VERSION: u8 = 1;
const MAX_CACHE_BYTES: u64 = 1024 * 1024;
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const QUERY_WINDOW_HOURS: i64 = 18;
// This permits substantial schedule revisions without accepting the adjacent
// day's ordinary occurrence of a same-number, same-route flight.
const MATCH_TOLERANCE_HOURS: i64 = 4;
const MAX_BACKOFF_SECONDS: u64 = 24 * 60 * 60;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlightStatus {
    pub provider: String,
    pub provider_flight_id: String,
    pub status: String,
    pub description: String,
    pub scheduled_departure: Option<String>,
    pub estimated_departure: Option<String>,
    pub actual_departure: Option<String>,
    pub scheduled_arrival: Option<String>,
    pub estimated_arrival: Option<String>,
    pub actual_arrival: Option<String>,
    pub departure_delay_seconds: Option<i64>,
    pub arrival_delay_seconds: Option<i64>,
    pub departure_terminal: Option<String>,
    pub departure_gate: Option<String>,
    pub arrival_terminal: Option<String>,
    pub arrival_gate: Option<String>,
    pub tracking_ended: bool,
    pub diverted: bool,
    pub current_position: Option<FlightPosition>,
    pub freshness: FlightFreshness,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlightFreshness {
    pub state: String,
    pub fetched_at: String,
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlightPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: String,
    pub altitude_feet: Option<i64>,
    pub groundspeed_knots: Option<i64>,
    pub heading_degrees: Option<i64>,
}

pub fn enrich_collection(config: &FlightAwareConfig, collection: &mut TravelCollection) {
    if !config.enabled || collection.legs.is_empty() {
        return;
    }

    let Some(api_key) = config.api_key.as_deref() else {
        push_warning(
            &mut collection.warnings,
            "flightaware_unconfigured",
            None,
            "FlightAware enrichment is enabled but flightaware.api_key is not configured",
        );
        return;
    };

    let cache_dir = match crate::cache::cache_directory() {
        Ok(path) => path.join("flightaware"),
        Err(error) => {
            push_warning(
                &mut collection.warnings,
                "flightaware_cache_unavailable",
                None,
                format!("FlightAware cache is unavailable: {error}"),
            );
            return;
        }
    };
    let transport = UreqTransport::new(config.request_timeout_seconds);
    enrich_with(
        config,
        collection,
        &transport,
        &cache_dir,
        Utc::now(),
        api_key,
    );
}

fn enrich_with(
    config: &FlightAwareConfig,
    collection: &mut TravelCollection,
    transport: &dyn Transport,
    cache_dir: &Path,
    now: DateTime<Utc>,
    api_key: &str,
) {
    let _usage_lock = match acquire_usage_lock(cache_dir) {
        Ok(lock) => lock,
        Err(error) => {
            push_warning(
                &mut collection.warnings,
                "flightaware_usage_unavailable",
                None,
                format!(
                    "FlightAware usage ledger is busy or unavailable; no provider requests were made: {error}"
                ),
            );
            return;
        }
    };
    let mut usage = match read_usage(cache_dir, now) {
        Ok(usage) => usage,
        Err(error) => {
            push_warning(
                &mut collection.warnings,
                "flightaware_usage_unavailable",
                None,
                format!(
                    "FlightAware usage ledger is unreadable; no provider requests were made: {error}"
                ),
            );
            return;
        }
    };

    let TravelCollection { legs, warnings } = collection;
    for leg in legs {
        let key = match CacheKey::from_leg(leg) {
            Ok(key) => key,
            Err(error) => {
                push_warning(
                    warnings,
                    "flightaware_invalid_leg",
                    Some(leg.source.event_id.clone()),
                    format!("{} cannot be enriched: {error}", leg.flight_number),
                );
                continue;
            }
        };
        let cache_path = cache_path(cache_dir, &key);
        let cached = match read_cache_entry(&cache_path, &key) {
            Ok(cached) => cached,
            Err(error) => {
                push_warning(
                    warnings,
                    "flightaware_cache_error",
                    Some(leg.source.event_id.clone()),
                    format!("{} cache entry was ignored: {error}", leg.flight_number),
                );
                None
            }
        };

        if let Some(entry) = cached.as_ref()
            && entry.is_fresh(now)
        {
            apply_fresh_cache(leg, entry, warnings);
            continue;
        }

        if usage.backoff_is_active(now) {
            use_stale_or_calendar(
                leg,
                cached.as_ref(),
                config.stale_if_error,
                warnings,
                "flightaware_backoff",
                "FlightAware requests are temporarily paused after a provider failure",
            );
            continue;
        }

        let Some((start, end)) = query_window(leg, now) else {
            use_stale_or_calendar(
                leg,
                cached.as_ref(),
                config.stale_if_error,
                warnings,
                "flightaware_outside_live_window",
                "FlightAware only exposes this endpoint for flights near the current date",
            );
            continue;
        };

        match reserve_result_set(cache_dir, &mut usage, config.monthly_result_set_limit) {
            Ok(true) => {}
            Ok(false) => {
                use_stale_or_calendar(
                    leg,
                    cached.as_ref(),
                    config.stale_if_error,
                    warnings,
                    "flightaware_monthly_limit",
                    "FlightAware monthly result-set limit has been reached",
                );
                continue;
            }
            Err(error) => {
                use_stale_or_calendar(
                    leg,
                    cached.as_ref(),
                    config.stale_if_error,
                    warnings,
                    "flightaware_usage_unavailable",
                    format!("FlightAware usage ledger could not be updated: {error}"),
                );
                continue;
            }
        }

        let response = transport.get(
            ProviderRequest::Flights {
                ident: leg.flight_number.clone(),
                start,
                end,
            },
            api_key,
        );
        let response = match response {
            Ok(response) if response.status == 200 => response,
            Ok(response) => {
                record_failure_or_warn(
                    cache_dir,
                    &mut usage,
                    now,
                    response.status,
                    response.retry_after,
                    warnings,
                    Some(leg.source.event_id.clone()),
                );
                use_stale_or_calendar(
                    leg,
                    cached.as_ref(),
                    config.stale_if_error,
                    warnings,
                    "flightaware_request_failed",
                    format!("FlightAware returned HTTP {}", response.status),
                );
                continue;
            }
            Err(()) => {
                record_failure_or_warn(
                    cache_dir,
                    &mut usage,
                    now,
                    0,
                    None,
                    warnings,
                    Some(leg.source.event_id.clone()),
                );
                use_stale_or_calendar(
                    leg,
                    cached.as_ref(),
                    config.stale_if_error,
                    warnings,
                    "flightaware_request_failed",
                    "FlightAware could not be reached",
                );
                continue;
            }
        };

        let provider: FlightsResponse = match serde_json::from_str(&response.body) {
            Ok(provider) => provider,
            Err(_) => {
                record_failure_or_warn(
                    cache_dir,
                    &mut usage,
                    now,
                    0,
                    None,
                    warnings,
                    Some(leg.source.event_id.clone()),
                );
                use_stale_or_calendar(
                    leg,
                    cached.as_ref(),
                    config.stale_if_error,
                    warnings,
                    "flightaware_invalid_response",
                    "FlightAware returned an invalid response",
                );
                continue;
            }
        };
        account_for_extra_pages(cache_dir, &mut usage, provider.num_pages);

        let matched = match select_match(&provider.flights, leg) {
            MatchSelection::Matched(flight) => flight,
            MatchSelection::NoMatch => {
                record_success(cache_dir, &mut usage);
                let entry = CacheEntry::negative(key.clone(), now, "no_match");
                if let Err(error) = write_cache_entry(&cache_path, &entry) {
                    push_cache_write_warning(warnings, leg, error);
                }
                push_warning(
                    warnings,
                    "flightaware_no_match",
                    Some(leg.source.event_id.clone()),
                    format!(
                        "FlightAware returned no unambiguous match for {} {}",
                        leg.flight_number, leg.route
                    ),
                );
                continue;
            }
            MatchSelection::Ambiguous => {
                record_success(cache_dir, &mut usage);
                let entry = CacheEntry::negative(key.clone(), now, "ambiguous");
                if let Err(error) = write_cache_entry(&cache_path, &entry) {
                    push_cache_write_warning(warnings, leg, error);
                }
                push_warning(
                    warnings,
                    "flightaware_ambiguous_match",
                    Some(leg.source.event_id.clone()),
                    format!(
                        "FlightAware returned multiple equally plausible matches for {} {}",
                        leg.flight_number, leg.route
                    ),
                );
                continue;
            }
        };

        let mut status = normalize_status(matched, now);
        let position_outcome = if flight_is_in_progress(matched) {
            enrich_position(
                config,
                transport,
                cache_dir,
                &mut usage,
                now,
                api_key,
                leg,
                matched,
                &mut status,
                warnings,
            )
        } else {
            PositionOutcome::Success
        };
        if position_outcome != PositionOutcome::Failure {
            record_success(cache_dir, &mut usage);
        }
        let expires_at = now + cache_ttl(&status, leg, now);
        status.freshness.expires_at = timestamp(expires_at);
        let entry = CacheEntry {
            version: CACHE_VERSION,
            key,
            fetched_at: timestamp(now),
            expires_at: timestamp(expires_at),
            outcome: "matched".to_string(),
            status: Some(status.clone()),
        };
        if let Err(error) = write_cache_entry(&cache_path, &entry) {
            push_cache_write_warning(warnings, leg, error);
        }
        leg.live_status = Some(status);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PositionOutcome {
    Success,
    Failure,
    Skipped,
}

#[allow(clippy::too_many_arguments)]
fn enrich_position(
    config: &FlightAwareConfig,
    transport: &dyn Transport,
    cache_dir: &Path,
    usage: &mut UsageState,
    now: DateTime<Utc>,
    api_key: &str,
    leg: &TravelLeg,
    matched: &ApiFlight,
    status: &mut FlightStatus,
    warnings: &mut Vec<TravelWarning>,
) -> PositionOutcome {
    match reserve_result_set(cache_dir, usage, config.monthly_result_set_limit) {
        Ok(true) => {}
        Ok(false) => {
            push_warning(
                warnings,
                "flightaware_position_skipped",
                Some(leg.source.event_id.clone()),
                "FlightAware position was skipped because the monthly result-set limit was reached",
            );
            return PositionOutcome::Skipped;
        }
        Err(error) => {
            push_warning(
                warnings,
                "flightaware_usage_unavailable",
                Some(leg.source.event_id.clone()),
                format!(
                    "FlightAware position was skipped because the usage ledger could not be updated: {error}"
                ),
            );
            return PositionOutcome::Skipped;
        }
    }

    let response = transport.get(
        ProviderRequest::Position {
            provider_flight_id: matched.fa_flight_id.clone(),
        },
        api_key,
    );
    match response {
        Ok(response) if response.status == 200 => {
            match serde_json::from_str::<PositionResponse>(&response.body) {
                Ok(PositionResponse {
                    last_position: None,
                }) => PositionOutcome::Success,
                Ok(PositionResponse {
                    last_position: Some(position),
                }) => match normalize_position(position) {
                    Some(position) => {
                        status.current_position = Some(position);
                        PositionOutcome::Success
                    }
                    None => {
                        record_failure_or_warn(
                            cache_dir,
                            usage,
                            now,
                            0,
                            None,
                            warnings,
                            Some(leg.source.event_id.clone()),
                        );
                        push_warning(
                            warnings,
                            "flightaware_invalid_position",
                            Some(leg.source.event_id.clone()),
                            "FlightAware returned invalid position values",
                        );
                        PositionOutcome::Failure
                    }
                },
                Err(_) => {
                    record_failure_or_warn(
                        cache_dir,
                        usage,
                        now,
                        0,
                        None,
                        warnings,
                        Some(leg.source.event_id.clone()),
                    );
                    push_warning(
                        warnings,
                        "flightaware_invalid_position",
                        Some(leg.source.event_id.clone()),
                        "FlightAware returned an invalid position response",
                    );
                    PositionOutcome::Failure
                }
            }
        }
        Ok(response) if response.status == 404 => PositionOutcome::Success,
        Ok(response) => {
            record_failure_or_warn(
                cache_dir,
                usage,
                now,
                response.status,
                response.retry_after,
                warnings,
                Some(leg.source.event_id.clone()),
            );
            push_warning(
                warnings,
                "flightaware_position_unavailable",
                Some(leg.source.event_id.clone()),
                format!("FlightAware position returned HTTP {}", response.status),
            );
            PositionOutcome::Failure
        }
        Err(()) => {
            record_failure_or_warn(
                cache_dir,
                usage,
                now,
                0,
                None,
                warnings,
                Some(leg.source.event_id.clone()),
            );
            push_warning(
                warnings,
                "flightaware_position_unavailable",
                Some(leg.source.event_id.clone()),
                "FlightAware position could not be reached",
            );
            PositionOutcome::Failure
        }
    }
}

fn apply_fresh_cache(leg: &mut TravelLeg, entry: &CacheEntry, warnings: &mut Vec<TravelWarning>) {
    if let Some(mut status) = entry.status.clone() {
        status.freshness.state = "fresh".to_string();
        status.freshness.fetched_at = entry.fetched_at.clone();
        status.freshness.expires_at = entry.expires_at.clone();
        leg.live_status = Some(status);
    } else {
        let (kind, message) = if entry.outcome == "ambiguous" {
            (
                "flightaware_ambiguous_match",
                format!(
                    "FlightAware returned multiple equally plausible matches for {} {}",
                    leg.flight_number, leg.route
                ),
            )
        } else {
            (
                "flightaware_no_match",
                format!(
                    "FlightAware returned no unambiguous match for {} {}",
                    leg.flight_number, leg.route
                ),
            )
        };
        push_warning(warnings, kind, Some(leg.source.event_id.clone()), message);
    }
}

fn use_stale_or_calendar(
    leg: &mut TravelLeg,
    cached: Option<&CacheEntry>,
    stale_if_error: bool,
    warnings: &mut Vec<TravelWarning>,
    kind: &str,
    message: impl Into<String>,
) {
    let message = message.into();
    if stale_if_error
        && let Some(entry) = cached
        && let Some(mut status) = entry.status.clone()
    {
        status.freshness.state = "stale".to_string();
        status.freshness.fetched_at = entry.fetched_at.clone();
        status.freshness.expires_at = entry.expires_at.clone();
        leg.live_status = Some(status);
        push_warning(
            warnings,
            "flightaware_stale_cache",
            Some(leg.source.event_id.clone()),
            format!("{}; showing stale cached status", message),
        );
    } else {
        push_warning(warnings, kind, Some(leg.source.event_id.clone()), message);
    }
}

fn push_cache_write_warning(
    warnings: &mut Vec<TravelWarning>,
    leg: &TravelLeg,
    error: anyhow::Error,
) {
    push_warning(
        warnings,
        "flightaware_cache_error",
        Some(leg.source.event_id.clone()),
        format!(
            "{} live status could not be cached: {error}",
            leg.flight_number
        ),
    );
}

fn push_warning(
    warnings: &mut Vec<TravelWarning>,
    kind: &str,
    event_id: Option<String>,
    message: impl Into<String>,
) {
    warnings.push(TravelWarning {
        kind: kind.to_string(),
        event_id,
        message: message.into(),
    });
}

fn query_window(leg: &TravelLeg, now: DateTime<Utc>) -> Option<(String, String)> {
    let departure = parse_timestamp(&leg.departure.utc)?;
    let earliest = now - Duration::days(9);
    let latest = now + Duration::hours(47);
    if departure < earliest || departure >= latest {
        return None;
    }
    let start = (departure - Duration::hours(QUERY_WINDOW_HOURS)).max(earliest);
    let end = (departure + Duration::hours(QUERY_WINDOW_HOURS)).min(latest);
    (start < departure && departure < end).then(|| (timestamp(start), timestamp(end)))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CacheKey {
    flight_number: String,
    origin: String,
    destination: String,
    departure_utc: String,
}

impl CacheKey {
    fn from_leg(leg: &TravelLeg) -> Result<Self> {
        let departure = parse_timestamp(&leg.departure.utc)
            .context("Calendar departure is not a valid RFC3339 timestamp")?;
        Ok(Self {
            flight_number: leg.flight_number.clone(),
            origin: leg.departure_airport.code.clone(),
            destination: leg.arrival_airport.code.clone(),
            departure_utc: timestamp(departure),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    version: u8,
    key: CacheKey,
    fetched_at: String,
    expires_at: String,
    outcome: String,
    status: Option<FlightStatus>,
}

impl CacheEntry {
    fn negative(key: CacheKey, now: DateTime<Utc>, outcome: &str) -> Self {
        Self {
            version: CACHE_VERSION,
            key,
            fetched_at: timestamp(now),
            expires_at: timestamp(now + Duration::minutes(30)),
            outcome: outcome.to_string(),
            status: None,
        }
    }

    fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        parse_timestamp(&self.expires_at).is_some_and(|expires_at| now < expires_at)
    }
}

fn cache_path(cache_dir: &Path, key: &CacheKey) -> PathBuf {
    let bytes = format!(
        "{}\0{}\0{}\0{}",
        key.flight_number, key.origin, key.destination, key.departure_utc
    );
    let hash = bytes.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    cache_dir.join(format!("flight-{hash:016x}.json"))
}

fn read_cache_entry(path: &Path, key: &CacheKey) -> Result<Option<CacheEntry>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("cache path is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_CACHE_BYTES {
        bail!("cache file exceeds {MAX_CACHE_BYTES} bytes");
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let entry: CacheEntry = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if entry.version != CACHE_VERSION || entry.key != *key {
        return Ok(None);
    }
    if parse_timestamp(&entry.fetched_at).is_none() || parse_timestamp(&entry.expires_at).is_none()
    {
        bail!("cache entry contains invalid timestamps");
    }
    Ok(Some(entry))
}

fn write_cache_entry(path: &Path, entry: &CacheEntry) -> Result<()> {
    write_json_atomically(path, entry)
}

#[derive(Debug, Serialize, Deserialize)]
struct UsageState {
    version: u8,
    month: String,
    result_sets: u32,
    failure_count: u32,
    backoff_until: Option<String>,
}

impl UsageState {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            version: USAGE_VERSION,
            month: month(now),
            result_sets: 0,
            failure_count: 0,
            backoff_until: None,
        }
    }

    fn backoff_is_active(&self, now: DateTime<Utc>) -> bool {
        self.backoff_until
            .as_deref()
            .and_then(parse_timestamp)
            .is_some_and(|until| now < until)
    }
}

fn read_usage(cache_dir: &Path, now: DateTime<Utc>) -> Result<UsageState> {
    let path = cache_dir.join("usage.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(UsageState::new(now)),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("usage ledger is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_CACHE_BYTES {
        bail!("usage ledger exceeds {MAX_CACHE_BYTES} bytes");
    }
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut usage: UsageState = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if usage.version != USAGE_VERSION {
        bail!("usage ledger has an unsupported version");
    }
    if usage.month != month(now) {
        usage = UsageState::new(now);
    }
    if usage
        .backoff_until
        .as_deref()
        .is_some_and(|value| parse_timestamp(value).is_none())
    {
        bail!("usage ledger contains an invalid backoff timestamp");
    }
    Ok(usage)
}

fn reserve_result_set(cache_dir: &Path, usage: &mut UsageState, limit: u32) -> Result<bool> {
    if usage.result_sets >= limit {
        return Ok(false);
    }
    usage.result_sets += 1;
    if let Err(error) = write_usage(cache_dir, usage) {
        usage.result_sets -= 1;
        return Err(error);
    }
    Ok(true)
}

fn account_for_extra_pages(cache_dir: &Path, usage: &mut UsageState, num_pages: u32) {
    if num_pages > 1 {
        usage.result_sets = usage.result_sets.saturating_add(num_pages - 1);
        let _ = write_usage(cache_dir, usage);
    }
}

fn record_success(cache_dir: &Path, usage: &mut UsageState) {
    usage.failure_count = 0;
    usage.backoff_until = None;
    let _ = write_usage(cache_dir, usage);
}

fn record_failure(
    cache_dir: &Path,
    usage: &mut UsageState,
    now: DateTime<Utc>,
    status: u16,
    retry_after: Option<u64>,
) -> Result<()> {
    usage.failure_count = usage.failure_count.saturating_add(1);
    let seconds = retry_after
        .unwrap_or_else(|| match status {
            401 | 403 => MAX_BACKOFF_SECONDS,
            429 => 10 * 60,
            _ => 60_u64
                .saturating_mul(2_u64.saturating_pow(usage.failure_count.saturating_sub(1).min(6)))
                .min(60 * 60),
        })
        .min(MAX_BACKOFF_SECONDS);
    let backoff_until = now
        .checked_add_signed(Duration::seconds(seconds as i64))
        .context("failed to calculate FlightAware backoff deadline")?;
    usage.backoff_until = Some(timestamp(backoff_until));
    write_usage(cache_dir, usage)
}

#[allow(clippy::too_many_arguments)]
fn record_failure_or_warn(
    cache_dir: &Path,
    usage: &mut UsageState,
    now: DateTime<Utc>,
    status: u16,
    retry_after: Option<u64>,
    warnings: &mut Vec<TravelWarning>,
    event_id: Option<String>,
) {
    if let Err(error) = record_failure(cache_dir, usage, now, status, retry_after) {
        push_warning(
            warnings,
            "flightaware_usage_unavailable",
            event_id,
            format!(
                "FlightAware backoff could not be persisted; provider requests are disabled for this refresh: {error}"
            ),
        );
    }
}

fn write_usage(cache_dir: &Path, usage: &UsageState) -> Result<()> {
    write_json_atomically(&cache_dir.join("usage.json"), usage)
}

fn acquire_usage_lock(cache_dir: &Path) -> Result<fs::File> {
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create cache directory {}", cache_dir.display()))?;
    let path = cache_dir.join("usage.lock");
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        bail!("usage lock is not a regular file: {}", path.display());
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    #[cfg(unix)]
    {
        let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if status != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("failed to lock {}", path.display()));
        }
    }
    Ok(file)
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .context("cache path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create cache directory {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(value).context("failed to serialize cache data")?;
    if bytes.len() as u64 > MAX_CACHE_BYTES {
        bail!("serialized cache data exceeds {MAX_CACHE_BYTES} bytes");
    }
    let counter = TEMP_FILE_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{counter}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let result = (|| -> Result<()> {
        let mut file = options
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn month(now: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", now.year(), now.month())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ProviderRequest {
    Flights {
        ident: String,
        start: String,
        end: String,
    },
    Position {
        provider_flight_id: String,
    },
}

struct ProviderResponse {
    status: u16,
    retry_after: Option<u64>,
    body: String,
}

trait Transport {
    fn get(
        &self,
        request: ProviderRequest,
        api_key: &str,
    ) -> std::result::Result<ProviderResponse, ()>;
}

struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new(timeout_seconds: u64) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(StdDuration::from_secs(timeout_seconds)))
            .https_only(true)
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(format!("icalctl/{}", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    fn response(
        &self,
        mut response: ureq::http::Response<ureq::Body>,
    ) -> std::result::Result<ProviderResponse, ()> {
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|_| ())?;
        Ok(ProviderResponse {
            status,
            retry_after,
            body,
        })
    }
}

impl Transport for UreqTransport {
    fn get(
        &self,
        request: ProviderRequest,
        api_key: &str,
    ) -> std::result::Result<ProviderResponse, ()> {
        let response = match request {
            ProviderRequest::Flights { ident, start, end } => self
                .agent
                .get(format!(
                    "{BASE_URL}/flights/{}",
                    encode_path_segment(&ident)
                ))
                .query("ident_type", "designator")
                .query("start", start)
                .query("end", end)
                .query("max_pages", "1")
                .header("accept", "application/json")
                .header("x-apikey", api_key)
                .call(),
            ProviderRequest::Position { provider_flight_id } => self
                .agent
                .get(format!(
                    "{BASE_URL}/flights/{}/position",
                    encode_path_segment(&provider_flight_id)
                ))
                .header("accept", "application/json")
                .header("x-apikey", api_key)
                .call(),
        }
        .map_err(|_| ())?;
        self.response(response)
    }
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
struct FlightsResponse {
    num_pages: u32,
    flights: Vec<ApiFlight>,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
struct ApiFlight {
    ident: String,
    ident_icao: Option<String>,
    ident_iata: Option<String>,
    fa_flight_id: String,
    #[serde(deserialize_with = "required_nullable")]
    codeshares: Option<Vec<String>>,
    codeshares_iata: Option<Vec<String>>,
    #[serde(deserialize_with = "required_nullable")]
    origin: Option<ApiAirport>,
    #[serde(deserialize_with = "required_nullable")]
    destination: Option<ApiAirport>,
    status: String,
    #[serde(deserialize_with = "required_nullable")]
    scheduled_out: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    estimated_out: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    actual_out: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    scheduled_off: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    estimated_off: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    actual_off: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    scheduled_on: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    estimated_on: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    actual_on: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    scheduled_in: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    estimated_in: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    actual_in: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    departure_delay: Option<i64>,
    #[serde(deserialize_with = "required_nullable")]
    arrival_delay: Option<i64>,
    #[serde(deserialize_with = "required_nullable")]
    terminal_origin: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    gate_origin: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    terminal_destination: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    gate_destination: Option<String>,
    cancelled: bool,
    diverted: bool,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
struct ApiAirport {
    #[serde(deserialize_with = "required_nullable")]
    code: Option<String>,
    code_icao: Option<String>,
    code_iata: Option<String>,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
struct PositionResponse {
    #[serde(deserialize_with = "required_nullable")]
    last_position: Option<ApiPosition>,
}

#[cfg_attr(test, derive(Serialize))]
#[derive(Debug, Deserialize)]
struct ApiPosition {
    latitude: f64,
    longitude: f64,
    timestamp: String,
    altitude: i64,
    groundspeed: i64,
    #[serde(deserialize_with = "required_nullable")]
    heading: Option<i64>,
}

fn required_nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

enum MatchSelection<'a> {
    Matched(&'a ApiFlight),
    NoMatch,
    Ambiguous,
}

fn select_match<'a>(flights: &'a [ApiFlight], leg: &TravelLeg) -> MatchSelection<'a> {
    let mut candidates = flights
        .iter()
        .filter_map(|flight| {
            let ident_rank = ident_rank(flight, &leg.flight_number)?;
            if !airport_matches(&leg.departure_airport, flight.origin.as_ref())
                || !airport_matches(&leg.arrival_airport, flight.destination.as_ref())
            {
                return None;
            }
            let provider_departure = provider_time(
                flight.scheduled_out.as_deref(),
                flight.scheduled_off.as_deref(),
            )?;
            let calendar_departure = parse_timestamp(&leg.departure.utc)?;
            let delta = (provider_departure - calendar_departure)
                .num_seconds()
                .unsigned_abs();
            (delta <= (MATCH_TOLERANCE_HOURS * 60 * 60) as u64)
                .then_some((flight, (delta, ident_rank)))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(compare_candidates);
    let Some((best, score)) = candidates.first() else {
        return MatchSelection::NoMatch;
    };
    if candidates
        .get(1)
        .is_some_and(|(_, next_score)| next_score == score)
    {
        return MatchSelection::Ambiguous;
    }
    MatchSelection::Matched(best)
}

fn compare_candidates(left: &(&ApiFlight, (u64, u8)), right: &(&ApiFlight, (u64, u8))) -> Ordering {
    left.1
        .cmp(&right.1)
        .then_with(|| left.0.fa_flight_id.cmp(&right.0.fa_flight_id))
}

fn ident_rank(flight: &ApiFlight, expected: &str) -> Option<u8> {
    let expected = normalize_ident(expected);
    [
        Some(flight.ident.as_str()),
        flight.ident_icao.as_deref(),
        flight.ident_iata.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|ident| normalize_ident(ident) == expected)
    .then_some(0)
    .or_else(|| {
        flight
            .codeshares
            .iter()
            .flatten()
            .chain(flight.codeshares_iata.iter().flatten())
            .any(|ident| normalize_ident(ident) == expected)
            .then_some(1)
    })
}

fn normalize_ident(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

fn airport_matches(calendar: &TravelAirport, provider: Option<&ApiAirport>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    let mut calendar_codes = vec![calendar.code.as_str()];
    if let Some(metadata) = &calendar.metadata {
        calendar_codes.push(&metadata.iata_code);
        if let Some(icao_code) = metadata.icao_code.as_deref() {
            calendar_codes.push(icao_code);
        }
    }
    let provider_codes = [
        provider.code.as_deref(),
        provider.code_icao.as_deref(),
        provider.code_iata.as_deref(),
    ];
    calendar_codes.iter().any(|calendar_code| {
        provider_codes
            .iter()
            .flatten()
            .any(|provider_code| calendar_code.eq_ignore_ascii_case(provider_code))
    })
}

fn normalize_status(flight: &ApiFlight, now: DateTime<Utc>) -> FlightStatus {
    let normalized = if flight.diverted {
        "diverted"
    } else if flight.actual_in.is_some() || flight.actual_on.is_some() {
        "arrived"
    } else if flight.cancelled {
        "tracking_ended"
    } else if flight.actual_out.is_some() || flight.actual_off.is_some() {
        "en_route"
    } else if flight.departure_delay.unwrap_or_default() > 15 * 60
        || flight.arrival_delay.unwrap_or_default() > 15 * 60
    {
        "delayed"
    } else {
        "scheduled"
    };
    let description = if flight.status.trim().is_empty() {
        normalized.replace('_', " ")
    } else {
        flight.status.clone()
    };
    FlightStatus {
        provider: PROVIDER.to_string(),
        provider_flight_id: flight.fa_flight_id.clone(),
        status: normalized.to_string(),
        description,
        scheduled_departure: normalized_time(
            flight.scheduled_out.as_deref(),
            flight.scheduled_off.as_deref(),
        ),
        estimated_departure: normalized_time(
            flight.estimated_out.as_deref(),
            flight.estimated_off.as_deref(),
        ),
        actual_departure: normalized_time(
            flight.actual_out.as_deref(),
            flight.actual_off.as_deref(),
        ),
        scheduled_arrival: normalized_time(
            flight.scheduled_in.as_deref(),
            flight.scheduled_on.as_deref(),
        ),
        estimated_arrival: normalized_time(
            flight.estimated_in.as_deref(),
            flight.estimated_on.as_deref(),
        ),
        actual_arrival: normalized_time(flight.actual_in.as_deref(), flight.actual_on.as_deref()),
        departure_delay_seconds: flight.departure_delay,
        arrival_delay_seconds: flight.arrival_delay,
        departure_terminal: flight.terminal_origin.clone(),
        departure_gate: flight.gate_origin.clone(),
        arrival_terminal: flight.terminal_destination.clone(),
        arrival_gate: flight.gate_destination.clone(),
        tracking_ended: flight.cancelled,
        diverted: flight.diverted,
        current_position: None,
        freshness: FlightFreshness {
            state: "fresh".to_string(),
            fetched_at: timestamp(now),
            expires_at: timestamp(now),
        },
    }
}

fn normalize_position(position: ApiPosition) -> Option<FlightPosition> {
    if !(-90.0..=90.0).contains(&position.latitude)
        || !(-180.0..=180.0).contains(&position.longitude)
        || position
            .heading
            .is_some_and(|heading| !(0..=360).contains(&heading))
    {
        return None;
    }
    let received_at = parse_timestamp(&position.timestamp)?;
    Some(FlightPosition {
        latitude: position.latitude,
        longitude: position.longitude,
        timestamp: timestamp(received_at),
        altitude_feet: position.altitude.checked_mul(100),
        groundspeed_knots: Some(position.groundspeed),
        heading_degrees: position.heading,
    })
}

fn provider_time(primary: Option<&str>, fallback: Option<&str>) -> Option<DateTime<Utc>> {
    primary
        .and_then(parse_timestamp)
        .or_else(|| fallback.and_then(parse_timestamp))
}

fn normalized_time(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    provider_time(primary, fallback).map(timestamp)
}

fn flight_is_in_progress(flight: &ApiFlight) -> bool {
    !flight.cancelled
        && (flight.actual_out.is_some() || flight.actual_off.is_some())
        && flight.actual_in.is_none()
        && flight.actual_on.is_none()
}

fn cache_ttl(status: &FlightStatus, leg: &TravelLeg, now: DateTime<Utc>) -> Duration {
    if matches!(status.status.as_str(), "arrived" | "tracking_ended") {
        return Duration::hours(6);
    }
    if status.status == "en_route" {
        return Duration::minutes(2);
    }
    let until_departure = parse_timestamp(&leg.departure.utc)
        .map(|departure| departure - now)
        .unwrap_or_else(Duration::zero);
    if until_departure <= Duration::hours(6) {
        Duration::minutes(5)
    } else if until_departure <= Duration::hours(24) {
        Duration::minutes(15)
    } else {
        Duration::hours(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::travel::{AirportMetadata, TravelAirport, TravelEventSource, TravelMoment};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!(
                "icalctl-flightaware-test-{}-{counter}",
                std::process::id()
            )))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct FakeTransport {
        responses: RefCell<VecDeque<std::result::Result<ProviderResponse, ()>>>,
        requests: RefCell<Vec<ProviderRequest>>,
        api_keys: RefCell<Vec<String>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<std::result::Result<ProviderResponse, ()>>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                requests: RefCell::new(Vec::new()),
                api_keys: RefCell::new(Vec::new()),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.borrow().len()
        }
    }

    impl Transport for FakeTransport {
        fn get(
            &self,
            request: ProviderRequest,
            api_key: &str,
        ) -> std::result::Result<ProviderResponse, ()> {
            self.requests.borrow_mut().push(request);
            self.api_keys.borrow_mut().push(api_key.to_string());
            self.responses.borrow_mut().pop_front().unwrap_or(Err(()))
        }
    }

    fn now() -> DateTime<Utc> {
        parse_timestamp("2026-07-11T00:00:00Z").unwrap()
    }

    fn config(limit: u32, stale_if_error: bool) -> FlightAwareConfig {
        FlightAwareConfig {
            api_key: Some("TEST-API-KEY".to_string()),
            enabled: true,
            monthly_result_set_limit: limit,
            request_timeout_seconds: 10,
            stale_if_error,
        }
    }

    fn airport(code: &str, iata: &str, icao: &str) -> TravelAirport {
        TravelAirport {
            code: code.to_string(),
            metadata: Some(AirportMetadata {
                iata_code: iata.to_string(),
                icao_code: Some(icao.to_string()),
                name: format!("{iata} Airport"),
                municipality: None,
                latitude: 0.0,
                longitude: 0.0,
            }),
        }
    }

    fn leg(
        event_id: &str,
        flight_number: &str,
        departure_utc: &str,
        arrival_utc: &str,
    ) -> TravelLeg {
        TravelLeg {
            flight_number: flight_number.to_string(),
            route: "PVG to HEL".to_string(),
            departure_airport: airport("PVG", "PVG", "ZSPD"),
            arrival_airport: airport("HEL", "HEL", "EFHK"),
            departure: TravelMoment {
                scheduled: departure_utc.to_string(),
                utc: departure_utc.to_string(),
            },
            arrival: TravelMoment {
                scheduled: arrival_utc.to_string(),
                utc: arrival_utc.to_string(),
            },
            live_status: None,
            source: TravelEventSource {
                event_id: event_id.to_string(),
                occurrence_date: None,
                title: format!("Flight {flight_number}: PVG to HEL"),
                calendar: Some("Travel".to_string()),
                calendar_id: Some("calendar-id".to_string()),
            },
        }
    }

    fn collection(legs: Vec<TravelLeg>) -> TravelCollection {
        TravelCollection {
            legs,
            warnings: Vec::new(),
        }
    }

    fn api_airport(code: &str, iata: &str, icao: &str) -> ApiAirport {
        ApiAirport {
            code: Some(code.to_string()),
            code_icao: Some(icao.to_string()),
            code_iata: Some(iata.to_string()),
        }
    }

    fn api_flight(
        provider_flight_id: &str,
        ident: &str,
        codeshares: &[&str],
        origin: (&str, &str, &str),
        destination: (&str, &str, &str),
        scheduled_out: &str,
    ) -> ApiFlight {
        ApiFlight {
            ident: ident.to_string(),
            ident_icao: Some(ident.to_string()),
            ident_iata: None,
            fa_flight_id: provider_flight_id.to_string(),
            codeshares: Some(codeshares.iter().map(|value| value.to_string()).collect()),
            codeshares_iata: None,
            origin: Some(api_airport(origin.0, origin.1, origin.2)),
            destination: Some(api_airport(destination.0, destination.1, destination.2)),
            status: "Scheduled".to_string(),
            scheduled_out: Some(scheduled_out.to_string()),
            estimated_out: None,
            actual_out: None,
            scheduled_off: None,
            estimated_off: None,
            actual_off: None,
            scheduled_on: None,
            estimated_on: None,
            actual_on: None,
            scheduled_in: Some("2026-07-11T11:00:00Z".to_string()),
            estimated_in: None,
            actual_in: None,
            departure_delay: None,
            arrival_delay: None,
            terminal_origin: Some("2".to_string()),
            gate_origin: Some("D71".to_string()),
            terminal_destination: Some("2".to_string()),
            gate_destination: Some("32".to_string()),
            cancelled: false,
            diverted: false,
        }
    }

    fn flights_response(flights: Vec<ApiFlight>) -> ProviderResponse {
        ProviderResponse {
            status: 200,
            retry_after: None,
            body: serde_json::to_string(&FlightsResponse {
                num_pages: 1,
                flights,
            })
            .unwrap(),
        }
    }

    fn scheduled_flight() -> ApiFlight {
        api_flight(
            "FA-HO1607",
            "DKH1607",
            &["HO1607"],
            ("ZSPD", "PVG", "ZSPD"),
            ("EFHK", "HEL", "EFHK"),
            "2026-07-11T01:25:00Z",
        )
    }

    fn in_progress_flight() -> ApiFlight {
        let mut flight = scheduled_flight();
        flight.status = "En Route".to_string();
        flight.actual_off = Some("2026-07-11T01:40:00Z".to_string());
        flight
    }

    #[test]
    fn matching_requires_ident_route_and_nearest_departure() {
        let travel_leg = leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        );
        let wrong_route = api_flight(
            "FA-WRONG-ROUTE",
            "HO1607",
            &[],
            ("ZSPD", "PVG", "ZSPD"),
            ("EDDF", "FRA", "EDDF"),
            "2026-07-11T01:25:00Z",
        );
        let wrong_time = api_flight(
            "FA-WRONG-TIME",
            "HO1607",
            &[],
            ("ZSPD", "PVG", "ZSPD"),
            ("EFHK", "HEL", "EFHK"),
            "2026-07-12T01:25:00Z",
        );
        let codeshare_match = scheduled_flight();

        let flights = [wrong_route, wrong_time, codeshare_match];
        let selection = select_match(&flights, &travel_leg);

        assert!(matches!(
            selection,
            MatchSelection::Matched(flight) if flight.fa_flight_id == "FA-HO1607"
        ));
    }

    #[test]
    fn equally_scored_provider_results_are_ambiguous() {
        let travel_leg = leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        );
        let first = api_flight(
            "FA-1",
            "HO1607",
            &[],
            ("ZSPD", "PVG", "ZSPD"),
            ("EFHK", "HEL", "EFHK"),
            "2026-07-11T01:25:00Z",
        );
        let second = api_flight(
            "FA-2",
            "HO1607",
            &[],
            ("ZSPD", "PVG", "ZSPD"),
            ("EFHK", "HEL", "EFHK"),
            "2026-07-11T01:25:00Z",
        );

        assert!(matches!(
            select_match(&[second, first], &travel_leg),
            MatchSelection::Ambiguous
        ));
    }

    #[test]
    fn sole_candidates_outside_four_hour_tolerance_are_rejected() {
        let travel_leg = leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        );
        let too_far = api_flight(
            "FA-TOO-FAR",
            "HO1607",
            &[],
            ("ZSPD", "PVG", "ZSPD"),
            ("EFHK", "HEL", "EFHK"),
            "2026-07-11T06:25:01Z",
        );
        assert!(matches!(
            select_match(&[too_far], &travel_leg),
            MatchSelection::NoMatch
        ));

        let within_tolerance = api_flight(
            "FA-WITHIN-TOLERANCE",
            "HO1607",
            &[],
            ("ZSPD", "PVG", "ZSPD"),
            ("EFHK", "HEL", "EFHK"),
            "2026-07-11T05:25:00Z",
        );
        assert!(matches!(
            select_match(&[within_tolerance], &travel_leg),
            MatchSelection::Matched(_)
        ));
    }

    #[test]
    fn nullable_codeshares_are_accepted_from_the_official_schema() {
        let mut value = serde_json::to_value(FlightsResponse {
            num_pages: 1,
            flights: vec![scheduled_flight()],
        })
        .unwrap();
        value["flights"][0]["codeshares"] = serde_json::Value::Null;
        value["flights"][0]["codeshares_iata"] = serde_json::Value::Null;

        let parsed: FlightsResponse = serde_json::from_value(value).unwrap();

        assert!(parsed.flights[0].codeshares.is_none());
        assert!(parsed.flights[0].codeshares_iata.is_none());
    }

    #[test]
    fn required_provider_response_fields_do_not_default_silently() {
        assert!(serde_json::from_str::<FlightsResponse>("{}").is_err());
        let valid = serde_json::to_value(FlightsResponse {
            num_pages: 1,
            flights: vec![scheduled_flight()],
        })
        .unwrap();
        for required in [
            "status",
            "cancelled",
            "diverted",
            "codeshares",
            "origin",
            "scheduled_out",
            "actual_in",
            "departure_delay",
            "gate_origin",
        ] {
            let mut missing = valid.clone();
            missing["flights"][0]
                .as_object_mut()
                .unwrap()
                .remove(required);
            assert!(
                serde_json::from_value::<FlightsResponse>(missing).is_err(),
                "missing required field {required} was accepted"
            );
        }
        for airport_field in ["origin", "destination"] {
            let mut nullable = valid.clone();
            nullable["flights"][0][airport_field]["code"] = serde_json::Value::Null;
            assert!(
                serde_json::from_value::<FlightsResponse>(nullable).is_ok(),
                "nullable {airport_field}.code was rejected"
            );

            let mut missing = valid.clone();
            missing["flights"][0][airport_field]
                .as_object_mut()
                .unwrap()
                .remove("code");
            assert!(
                serde_json::from_value::<FlightsResponse>(missing).is_err(),
                "missing required field {airport_field}.code was accepted"
            );
        }
        assert!(serde_json::from_str::<PositionResponse>("{}").is_err());
        assert!(
            serde_json::from_value::<PositionResponse>(serde_json::json!({
                "last_position": {
                    "latitude": 55.5,
                    "longitude": 42.25,
                    "timestamp": "2026-07-11T02:15:00Z",
                    "heading": null
                }
            }))
            .is_err()
        );

        let out_of_range = ApiPosition {
            latitude: 55.5,
            longitude: 42.25,
            timestamp: "2026-07-11T02:15:00Z".to_string(),
            altitude: 330,
            groundspeed: 455,
            heading: Some(361),
        };
        assert!(normalize_position(out_of_range).is_none());
    }

    #[test]
    fn authenticated_transport_never_follows_redirects() {
        let transport = UreqTransport::new(10);

        assert_eq!(transport.agent.config().max_redirects(), 0);
    }

    #[test]
    fn oversized_retry_after_is_capped_without_panicking() {
        let directory = TestDirectory::new();
        let mut usage = UsageState::new(now());

        record_failure(directory.path(), &mut usage, now(), 429, Some(u64::MAX)).unwrap();

        assert_eq!(
            parse_timestamp(usage.backoff_until.as_deref().unwrap()).unwrap() - now(),
            Duration::hours(24)
        );
    }

    #[test]
    fn fresh_normalized_cache_prevents_repeat_provider_calls() {
        let directory = TestDirectory::new();
        let first_transport =
            FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
        let mut first = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);

        enrich_with(
            &config(10, true),
            &mut first,
            &first_transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );
        assert_eq!(first_transport.request_count(), 1);
        assert_eq!(
            first.legs[0].live_status.as_ref().unwrap().status,
            "scheduled"
        );

        let cached_transport = FakeTransport::new(Vec::new());
        let mut cached = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);
        enrich_with(
            &config(10, true),
            &mut cached,
            &cached_transport,
            directory.path(),
            now() + Duration::minutes(1),
            "TEST-API-KEY",
        );

        assert_eq!(cached_transport.request_count(), 0);
        assert_eq!(
            cached.legs[0].live_status.as_ref().unwrap().freshness.state,
            "fresh"
        );
    }

    #[test]
    fn monthly_limit_blocks_additional_result_sets() {
        let directory = TestDirectory::new();
        let transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
        let mut travel = collection(vec![
            leg(
                "event-1",
                "HO1607",
                "2026-07-11T01:25:00Z",
                "2026-07-11T11:00:00Z",
            ),
            leg(
                "event-2",
                "HO1608",
                "2026-07-11T03:25:00Z",
                "2026-07-11T13:00:00Z",
            ),
        ]);

        enrich_with(
            &config(1, true),
            &mut travel,
            &transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );

        assert_eq!(transport.request_count(), 1);
        assert!(
            travel
                .warnings
                .iter()
                .any(|warning| warning.kind == "flightaware_monthly_limit")
        );
        assert_eq!(read_usage(directory.path(), now()).unwrap().result_sets, 1);
    }

    #[test]
    fn provider_backoff_prevents_follow_on_calls() {
        let directory = TestDirectory::new();
        let transport = FakeTransport::new(vec![Ok(ProviderResponse {
            status: 429,
            retry_after: Some(600),
            body: String::new(),
        })]);
        let mut travel = collection(vec![
            leg(
                "event-1",
                "HO1607",
                "2026-07-11T01:25:00Z",
                "2026-07-11T11:00:00Z",
            ),
            leg(
                "event-2",
                "HO1608",
                "2026-07-11T03:25:00Z",
                "2026-07-11T13:00:00Z",
            ),
        ]);

        enrich_with(
            &config(10, true),
            &mut travel,
            &transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );

        assert_eq!(transport.request_count(), 1);
        assert!(
            travel
                .warnings
                .iter()
                .any(|warning| warning.kind == "flightaware_backoff")
        );
        assert!(
            read_usage(directory.path(), now())
                .unwrap()
                .backoff_is_active(now())
        );
    }

    #[test]
    fn backoff_persistence_errors_are_explicit_and_fail_closed_in_memory() {
        let directory = TestDirectory::new();
        fs::create_dir_all(directory.path().join("usage.json")).unwrap();
        let mut usage = UsageState::new(now());
        let mut warnings = Vec::new();

        record_failure_or_warn(
            directory.path(),
            &mut usage,
            now(),
            503,
            None,
            &mut warnings,
            Some("event-1".to_string()),
        );

        assert!(usage.backoff_is_active(now()));
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, "flightaware_usage_unavailable");
        assert!(read_usage(directory.path(), now()).is_err());
    }

    #[test]
    fn expired_cache_is_used_only_when_stale_fallback_is_enabled() {
        let directory = TestDirectory::new();
        let travel_leg = leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        );
        let key = CacheKey::from_leg(&travel_leg).unwrap();
        let mut status = normalize_status(&scheduled_flight(), now() - Duration::hours(2));
        status.freshness.expires_at = timestamp(now() - Duration::hours(1));
        let entry = CacheEntry {
            version: CACHE_VERSION,
            key: key.clone(),
            fetched_at: timestamp(now() - Duration::hours(2)),
            expires_at: timestamp(now() - Duration::hours(1)),
            outcome: "matched".to_string(),
            status: Some(status),
        };
        write_cache_entry(&cache_path(directory.path(), &key), &entry).unwrap();

        let transport = FakeTransport::new(vec![Err(())]);
        let mut with_stale = collection(vec![travel_leg]);
        enrich_with(
            &config(10, true),
            &mut with_stale,
            &transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );

        assert_eq!(
            with_stale.legs[0]
                .live_status
                .as_ref()
                .unwrap()
                .freshness
                .state,
            "stale"
        );
        assert!(
            with_stale
                .warnings
                .iter()
                .any(|warning| warning.kind == "flightaware_stale_cache")
        );

        let later = now() + Duration::hours(2);
        let mut without_stale = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);
        let transport = FakeTransport::new(vec![Err(())]);
        let separate_directory = TestDirectory::new();
        let key = CacheKey::from_leg(&without_stale.legs[0]).unwrap();
        write_cache_entry(
            &cache_path(separate_directory.path(), &key),
            &CacheEntry {
                version: CACHE_VERSION,
                key,
                fetched_at: timestamp(later - Duration::hours(2)),
                expires_at: timestamp(later - Duration::hours(1)),
                outcome: "matched".to_string(),
                status: Some(normalize_status(
                    &scheduled_flight(),
                    later - Duration::hours(2),
                )),
            },
        )
        .unwrap();
        enrich_with(
            &config(10, false),
            &mut without_stale,
            &transport,
            separate_directory.path(),
            later,
            "TEST-API-KEY",
        );
        assert!(without_stale.legs[0].live_status.is_none());
    }

    #[test]
    fn in_progress_match_fetches_and_normalizes_current_position() {
        let directory = TestDirectory::new();
        let position = PositionResponse {
            last_position: Some(ApiPosition {
                latitude: 55.5,
                longitude: 42.25,
                timestamp: "2026-07-11T02:15:00Z".to_string(),
                altitude: 330,
                groundspeed: 455,
                heading: Some(310),
            }),
        };
        let transport = FakeTransport::new(vec![
            Ok(flights_response(vec![in_progress_flight()])),
            Ok(ProviderResponse {
                status: 200,
                retry_after: None,
                body: serde_json::to_string(&position).unwrap(),
            }),
        ]);
        let mut travel = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);

        enrich_with(
            &config(10, true),
            &mut travel,
            &transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );

        assert_eq!(transport.request_count(), 2);
        let status = travel.legs[0].live_status.as_ref().unwrap();
        assert_eq!(status.status, "en_route");
        let position = status.current_position.as_ref().unwrap();
        assert_eq!(position.altitude_feet, Some(33_000));
        assert_eq!(position.groundspeed_knots, Some(455));
    }

    #[test]
    fn position_failures_accumulate_backoff_across_summary_successes() {
        let directory = TestDirectory::new();
        let mut first = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);
        let first_transport = FakeTransport::new(vec![
            Ok(flights_response(vec![in_progress_flight()])),
            Err(()),
        ]);
        enrich_with(
            &config(10, true),
            &mut first,
            &first_transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );
        assert_eq!(first_transport.request_count(), 2);
        assert_eq!(
            read_usage(directory.path(), now()).unwrap().failure_count,
            1
        );

        let second_now = now() + Duration::minutes(2) + Duration::seconds(1);
        let mut second = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);
        let second_transport = FakeTransport::new(vec![
            Ok(flights_response(vec![in_progress_flight()])),
            Err(()),
        ]);
        enrich_with(
            &config(10, true),
            &mut second,
            &second_transport,
            directory.path(),
            second_now,
            "TEST-API-KEY",
        );

        assert_eq!(second_transport.request_count(), 2);
        let usage = read_usage(directory.path(), second_now).unwrap();
        assert_eq!(usage.failure_count, 2);
        assert!(usage.backoff_is_active(second_now));
    }

    #[test]
    fn invalid_position_payload_enters_provider_backoff() {
        let directory = TestDirectory::new();
        let transport = FakeTransport::new(vec![
            Ok(flights_response(vec![in_progress_flight()])),
            Ok(ProviderResponse {
                status: 200,
                retry_after: None,
                body: "not-json".to_string(),
            }),
        ]);
        let mut travel = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);

        enrich_with(
            &config(10, true),
            &mut travel,
            &transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );

        assert!(
            read_usage(directory.path(), now())
                .unwrap()
                .backoff_is_active(now())
        );
        assert!(
            travel
                .warnings
                .iter()
                .any(|warning| warning.kind == "flightaware_invalid_position")
        );
    }

    #[test]
    fn provider_tracking_end_flag_is_not_presented_as_confirmed_cancellation() {
        let mut tracking_ended = scheduled_flight();
        tracking_ended.cancelled = true;
        tracking_ended.status = "No longer tracked".to_string();
        let normalized = normalize_status(&tracking_ended, now());
        assert_eq!(normalized.status, "tracking_ended");
        assert!(normalized.tracking_ended);

        let mut arrived = tracking_ended;
        arrived.actual_in = Some("2026-07-11T11:02:00Z".to_string());
        assert_eq!(normalize_status(&arrived, now()).status, "arrived");

        let mut diverted = arrived;
        diverted.diverted = true;
        assert_eq!(normalize_status(&diverted, now()).status, "diverted");
    }

    #[test]
    fn provider_key_never_enters_output_warnings_or_cache_files() {
        let directory = TestDirectory::new();
        let secret = "SUPER-SECRET-FLIGHTAWARE-KEY";
        let transport = FakeTransport::new(vec![Ok(ProviderResponse {
            status: 401,
            retry_after: None,
            body: format!("provider echoed {secret}"),
        })]);
        let mut travel = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);

        enrich_with(
            &config(10, true),
            &mut travel,
            &transport,
            directory.path(),
            now(),
            secret,
        );

        assert_eq!(transport.api_keys.borrow().as_slice(), [secret]);
        assert!(!serde_json::to_string(&travel).unwrap().contains(secret));
        for entry in fs::read_dir(directory.path()).unwrap() {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains(secret));
        }
    }

    #[test]
    fn cache_files_are_owner_only_on_unix() {
        let directory = TestDirectory::new();
        let path = directory.path().join("entry.json");
        write_json_atomically(&path, &UsageState::new(now())).unwrap();

        #[cfg(unix)]
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_busy_usage_ledger_fails_closed_without_provider_calls() {
        let directory = TestDirectory::new();
        let _lock = acquire_usage_lock(directory.path()).unwrap();
        let transport = FakeTransport::new(vec![Ok(flights_response(vec![scheduled_flight()]))]);
        let mut travel = collection(vec![leg(
            "event-1",
            "HO1607",
            "2026-07-11T01:25:00Z",
            "2026-07-11T11:00:00Z",
        )]);

        enrich_with(
            &config(10, true),
            &mut travel,
            &transport,
            directory.path(),
            now(),
            "TEST-API-KEY",
        );

        assert_eq!(transport.request_count(), 0);
        assert!(
            travel
                .warnings
                .iter()
                .any(|warning| warning.kind == "flightaware_usage_unavailable")
        );
    }

    #[test]
    fn far_future_flights_do_not_consume_provider_quota() {
        let travel_leg = leg(
            "event-future",
            "HO1607",
            "2026-07-20T01:25:00Z",
            "2026-07-20T11:00:00Z",
        );

        assert!(query_window(&travel_leg, now()).is_none());
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_path_segment("HO 16/07"), "HO%2016%2F07");
    }
}
