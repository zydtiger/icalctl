use super::{parse_timestamp, timestamp};
use crate::travel::{FlightStatus, TravelLeg};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub(super) const CACHE_VERSION: u8 = 1;
pub(super) const MAX_CACHE_BYTES: u64 = 1024 * 1024;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CacheKey {
    pub(super) flight_number: String,
    pub(super) origin: String,
    pub(super) destination: String,
    pub(super) departure_utc: String,
}

impl CacheKey {
    pub(super) fn from_leg(leg: &TravelLeg) -> Result<Self> {
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
pub(super) struct CacheEntry {
    pub(super) version: u8,
    pub(super) key: CacheKey,
    pub(super) fetched_at: String,
    pub(super) expires_at: String,
    pub(super) outcome: String,
    pub(super) status: Option<FlightStatus>,
}

impl CacheEntry {
    pub(super) fn negative(key: CacheKey, now: DateTime<Utc>, outcome: &str) -> Self {
        Self {
            version: CACHE_VERSION,
            key,
            fetched_at: timestamp(now),
            expires_at: timestamp(now + Duration::minutes(30)),
            outcome: outcome.to_string(),
            status: None,
        }
    }

    pub(super) fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        parse_timestamp(&self.expires_at).is_some_and(|expires_at| now < expires_at)
    }
}

pub(super) fn cache_path(cache_dir: &Path, key: &CacheKey) -> PathBuf {
    let bytes = format!(
        "{}\0{}\0{}\0{}",
        key.flight_number, key.origin, key.destination, key.departure_utc
    );
    let hash = bytes.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    cache_dir.join(format!("flight-{hash:016x}.json"))
}

pub(super) fn read_cache_entry(path: &Path, key: &CacheKey) -> Result<Option<CacheEntry>> {
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

pub(super) fn write_cache_entry(path: &Path, entry: &CacheEntry) -> Result<()> {
    write_json_atomically(path, entry)
}

pub(super) fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
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
