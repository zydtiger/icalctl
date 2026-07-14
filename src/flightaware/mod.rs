mod cache;
mod client;
mod matching;
mod normalization;
mod quota;
#[cfg(test)]
mod tests;

use self::cache::*;
use self::client::*;
use self::matching::*;
use self::normalization::*;
use self::quota::*;
use crate::config::FlightAwareConfig;
use crate::travel::{FlightStatus, TravelCollection, TravelLeg, TravelWarning};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use std::path::Path;

const QUERY_WINDOW_HOURS: i64 = 18;

pub fn enrich_collection(config: &FlightAwareConfig, collection: &mut TravelCollection) {
    if !config.enabled || collection.legs.is_empty() {
        return;
    }
    let now = Utc::now();
    if !has_queryable_leg(collection, now) {
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
    enrich_with(config, collection, &transport, &cache_dir, now, api_key);
}

fn enrich_with(
    config: &FlightAwareConfig,
    collection: &mut TravelCollection,
    transport: &dyn Transport,
    cache_dir: &Path,
    now: DateTime<Utc>,
    api_key: &str,
) {
    if !has_queryable_leg(collection, now) {
        return;
    }

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
        let Some((start, end)) = query_window(leg, now) else {
            continue;
        };

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
                ident: normalize_ident(&leg.flight_number),
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

fn has_queryable_leg(collection: &TravelCollection, now: DateTime<Utc>) -> bool {
    collection
        .legs
        .iter()
        .any(|leg| query_window(leg, now).is_some())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}
