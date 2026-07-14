mod batch;
mod dispatch;
mod duplicates;
mod eventkit;
mod mutations;
mod read;
mod recurrence;
mod selection;
#[cfg(test)]
mod tests;

pub use self::dispatch::run;

pub(crate) use self::eventkit::{
    create_event_in_calendar, read_event_details, update_event_calendar_metadata,
    validate_event_time_zone, validate_event_url,
};
pub(crate) use self::read::{authorized_events_manager, ensure_valid_event_range, fetch_events};
pub(crate) use self::recurrence::{
    parse_event_recurrence, recurrence_rules_match, validate_recurring_all_day_inputs,
};
pub(crate) use self::selection::{resolve_target_calendar, resolve_target_calendar_with_selection};

use self::duplicates::*;
use self::eventkit::*;
use self::read::*;
use self::recurrence::*;
use self::selection::*;

use self::selection::{CalendarSelector, resolve_calendars};
use crate::cache::resolve_event_show_ref;
use crate::cli::{
    AvailabilityArg, BatchCommand, Command, EventJsonRecurrence, EventRecurrenceArgs,
    EventRepeatArg, EventScopeArg, EventWeekdayArg, IfExistsArg, ReadCalendarSelectorArgs,
    TravelCommand, WriteCalendarSelectorArgs,
};
use crate::dates::{
    datetime_in_time_zone, parse_end_datetime, parse_end_datetime_in_time_zone,
    parse_start_datetime, parse_start_datetime_in_time_zone, today_range, utc_datetime,
    validate_time_zone,
};

use crate::models::{
    CalendarReport, CalendarSelection, DeletedReport, EventDraftReport, EventRecurrenceEndReport,
    EventRecurrenceReport, EventRecurrenceWeekdayReport, EventReport, JsonOutput, StatusReport,
};
use crate::output::event_time_range;
use ::eventkit::{
    AlarmInfo, AlarmProximity, AuthorizationStatus, CalendarInfo, EventAvailability, EventDraft,
    EventItem, EventKitError, EventPatch, EventSpan, EventsManager,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use serde::Deserialize;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use self::batch::*;
use self::mutations::*;

struct AddCommandInput {
    title: Option<String>,
    start: Option<String>,
    end: Option<String>,
    calendar_selector: WriteCalendarSelectorArgs,
    notes: Option<String>,
    notes_file: Option<PathBuf>,
    json_file: Option<PathBuf>,
    location: Option<String>,
    url: Option<String>,
    all_day: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    alarm_minutes_before: Vec<i64>,
    recurrence: EventRecurrenceArgs,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

struct AddEventInput {
    title: String,
    start: String,
    end: String,
    calendar_selector: WriteCalendarSelectorArgs,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    all_day: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    alarm_minutes_before: Vec<i64>,
    recurrence: Option<EventRecurrenceReport>,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonAddDraft {
    title: String,
    start: String,
    end: String,
    calendar: Option<String>,
    calendar_id: Option<String>,
    calendar_source: Option<String>,
    source_id: Option<String>,
    notes: Option<String>,
    location: Option<String>,
    url: Option<String>,
    #[serde(default)]
    all_day: bool,
    #[serde(default)]
    timed: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    #[serde(default)]
    alarm_minutes_before: Vec<i64>,
    recurrence: Option<EventJsonRecurrence>,
}

struct UpdateEventInput {
    id: String,
    occurrence_start: Option<String>,
    scope: Option<EventScopeArg>,
    title: Option<String>,
    start: Option<String>,
    end: Option<String>,
    calendar_selector: WriteCalendarSelectorArgs,
    notes: Option<String>,
    clear_notes: bool,
    location: Option<String>,
    clear_location: bool,
    url: Option<String>,
    clear_url: bool,
    all_day: bool,
    timed: bool,
    availability: Option<AvailabilityArg>,
    time_zone: Option<String>,
    clear_time_zone: bool,
    add_alarm_minutes_before: Vec<i64>,
    dry_run: bool,
}

enum WriteEventResult {
    Written(Box<EventReport>),
    DryRun(Box<EventDraftReport>),
}
