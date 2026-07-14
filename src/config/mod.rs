mod commands;
mod schema;
#[path = "io.rs"]
mod secure_io;
#[cfg(test)]
mod tests;
mod validation;

pub use self::commands::run;
pub use self::schema::{Config, FlightAwareConfig, MAX_TRAVEL_RANGE_DAYS};
#[cfg(test)]
pub(crate) use self::secure_io::with_test_config_contents;

#[cfg(test)]
use self::commands::*;
use self::schema::*;
use self::secure_io::*;
use self::validation::*;

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
