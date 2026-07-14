mod batch;
mod dispatch;
mod eventkit;
mod mutation_support;
mod mutations;
mod query;
mod schedule;
mod selection;
mod store;
#[cfg(test)]
mod tests;

pub use self::dispatch::run;
pub(crate) use self::eventkit::ReminderAuthorization;

use self::batch::*;
use self::eventkit::*;
use self::mutation_support::*;
use self::mutations::*;
use self::query::*;
use self::schedule::*;
use self::selection::*;
use self::store::*;

use crate::cache::resolve_reminder_ref;
use crate::cli::{
    IfExistsArg, ReminderAdvancedScheduleArgs, ReminderBatchCommand, ReminderGeofenceProximityArg,
    ReminderPriorityArg, ReminderRepeatArg, RemindersCommand, WriteReminderListSelectorArgs,
};
use crate::dates::validate_time_zone;
use crate::models::{
    BatchErrorReport, BatchSummaryReport, JsonOutput, ReminderBatchItemReport, ReminderBatchReport,
    ReminderDateKind, ReminderDateReport, ReminderDeletedReport, ReminderDraftReport,
    ReminderListReport, ReminderListSelection, ReminderMutationDraftReport, ReminderReport,
    StatusReport,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, NaiveDate, Utc};
use objc2_foundation::{NSString, NSURL};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io::Write as IoWrite;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::cli::{ReminderReadFilterArgs, ReminderStateArg};
#[cfg(test)]
use crate::models::ReminderPriority;
#[cfg(test)]
use chrono::Local;
#[cfg(test)]
use objc2_event_kit::{EKAuthorizationStatus, EKEventStore};
#[cfg(test)]
use objc2_foundation::NSDateComponents;

struct AddReminderCliCommand {
    title: Option<String>,
    json_file: Option<PathBuf>,
    list_selector: WriteReminderListSelectorArgs,
    parent_id: Option<String>,
    due: Option<String>,
    start: Option<String>,
    time_zone: Option<String>,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    url: Option<String>,
    location: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: bool,
    notify_minutes_before: Vec<i64>,
    schedule: ReminderAdvancedScheduleArgs,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Clone)]
struct AddReminderCommand {
    title: String,
    list_selector: WriteReminderListSelectorArgs,
    parent_id: Option<String>,
    due: Option<String>,
    start: Option<String>,
    time_zone: Option<String>,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    url: Option<String>,
    location: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: bool,
    notify_minutes_before: Vec<i64>,
    schedule: ReminderAdvancedScheduleArgs,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Clone)]
struct ReminderSaveDraft {
    title: String,
    list_id: String,
    parent_id: Option<String>,
    due: Option<ParsedReminderDate>,
    start: Option<ParsedReminderDate>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    priority_value: usize,
    notifications: Vec<ParsedReminderNotification>,
    geofence: Option<ParsedReminderGeofence>,
    recurrence: Option<ParsedReminderRecurrence>,
}

#[derive(Clone, Default)]
struct ReminderAddPatch {
    parent_id: Option<String>,
    start: Option<ParsedReminderDate>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    priority_value: Option<usize>,
    notifications: Option<Vec<ParsedReminderNotification>>,
    geofence: Option<Option<ParsedReminderGeofence>>,
    recurrence: Option<ParsedReminderRecurrence>,
}

struct UpdateReminderCommand {
    id: String,
    title: Option<String>,
    list_selector: WriteReminderListSelectorArgs,
    parent_id: Option<String>,
    clear_parent: bool,
    due: Option<String>,
    clear_due: bool,
    start: Option<String>,
    clear_start: bool,
    time_zone: Option<String>,
    clear_time_zone: bool,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    clear_notes: bool,
    url: Option<String>,
    clear_url: bool,
    location: Option<String>,
    clear_location: bool,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: bool,
    notify_minutes_before: Vec<i64>,
    schedule: ReminderAdvancedScheduleArgs,
    clear_notifications: bool,
    clear_recurrence: bool,
    dry_run: bool,
}

#[derive(Clone, Default)]
struct ReminderLifecyclePatch {
    title: Option<String>,
    list_id: Option<String>,
    parent_id: Option<Option<String>>,
    due: Option<Option<ParsedReminderDate>>,
    start: Option<Option<ParsedReminderDate>>,
    notes: Option<Option<String>>,
    location: Option<Option<String>>,
    url: Option<Option<String>>,
    priority_value: Option<usize>,
    notifications: Option<Vec<ParsedReminderNotification>>,
    geofence: Option<Option<ParsedReminderGeofence>>,
    recurrence: Option<Option<ParsedReminderRecurrence>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderJsonGeofence {
    title: String,
    latitude: f64,
    longitude: f64,
    radius_meters: f64,
    proximity: ReminderGeofenceProximityArg,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderJsonRecurrence {
    frequency: ReminderRepeatArg,
    interval: Option<usize>,
    count: Option<usize>,
    until: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderJsonDraft {
    client_id: Option<String>,
    title: String,
    list: Option<String>,
    list_id: Option<String>,
    list_source: Option<String>,
    source_id: Option<String>,
    parent_id: Option<String>,
    due: Option<String>,
    start: Option<String>,
    #[serde(default)]
    time_zone: ReminderJsonOverride<String>,
    notes: Option<String>,
    url: Option<String>,
    location: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: Option<bool>,
    notify_minutes_before: Option<Vec<i64>>,
    notify_at: Option<Vec<String>>,
    #[serde(default)]
    geofence: ReminderJsonOverride<ReminderJsonGeofence>,
    #[serde(default)]
    recurrence: ReminderJsonOverride<ReminderJsonRecurrence>,
}

#[derive(Clone, Debug, Default)]
enum ReminderJsonOverride<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for ReminderJsonOverride<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderBatchDefaults {
    list: Option<String>,
    list_id: Option<String>,
    list_source: Option<String>,
    source_id: Option<String>,
    parent_id: Option<String>,
    time_zone: Option<String>,
    priority: Option<ReminderPriorityArg>,
    notify_at_due: Option<bool>,
    notify_minutes_before: Option<Vec<i64>>,
    notify_at: Option<Vec<String>>,
    geofence: Option<ReminderJsonGeofence>,
    recurrence: Option<ReminderJsonRecurrence>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReminderBatchEnvelope {
    version: u8,
    #[serde(default)]
    defaults: ReminderBatchDefaults,
    reminders: Vec<Value>,
}
