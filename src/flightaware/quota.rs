use super::cache::{MAX_CACHE_BYTES, write_json_atomically};
use super::{parse_timestamp, push_warning, timestamp};
use crate::travel::TravelWarning;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const USAGE_VERSION: u8 = 1;
const MAX_BACKOFF_SECONDS: u64 = 24 * 60 * 60;

fn month(now: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", now.year(), now.month())
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct UsageState {
    pub(super) version: u8,
    pub(super) month: String,
    pub(super) result_sets: u32,
    pub(super) failure_count: u32,
    pub(super) backoff_until: Option<String>,
}

impl UsageState {
    pub(super) fn new(now: DateTime<Utc>) -> Self {
        Self {
            version: USAGE_VERSION,
            month: month(now),
            result_sets: 0,
            failure_count: 0,
            backoff_until: None,
        }
    }

    pub(super) fn backoff_is_active(&self, now: DateTime<Utc>) -> bool {
        self.backoff_until
            .as_deref()
            .and_then(parse_timestamp)
            .is_some_and(|until| now < until)
    }
}

pub(super) fn read_usage(cache_dir: &Path, now: DateTime<Utc>) -> Result<UsageState> {
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

pub(super) fn reserve_result_set(
    cache_dir: &Path,
    usage: &mut UsageState,
    limit: u32,
) -> Result<bool> {
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

pub(super) fn account_for_extra_pages(cache_dir: &Path, usage: &mut UsageState, num_pages: u32) {
    if num_pages > 1 {
        usage.result_sets = usage.result_sets.saturating_add(num_pages - 1);
        let _ = write_usage(cache_dir, usage);
    }
}

pub(super) fn record_success(cache_dir: &Path, usage: &mut UsageState) {
    usage.failure_count = 0;
    usage.backoff_until = None;
    let _ = write_usage(cache_dir, usage);
}

pub(super) fn record_failure(
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
pub(super) fn record_failure_or_warn(
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

pub(super) fn acquire_usage_lock(cache_dir: &Path) -> Result<fs::File> {
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
