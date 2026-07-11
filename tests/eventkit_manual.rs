use chrono::DateTime;
use serde_json::Value;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const ICALCTL: &str = env!("CARGO_BIN_EXE_icalctl");
const TEST_CALENDAR_TITLE: &str = "icalctl Test";
const TEST_REMINDER_LIST_TITLE: &str = "icalctl Test";

fn run(args: &[&str]) -> Output {
    Command::new(ICALCTL)
        .args(args)
        .output()
        .expect("failed to run icalctl test binary")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "failed to parse command JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

struct EventCleanup {
    id: Option<String>,
}

impl EventCleanup {
    fn delete(&mut self) -> Output {
        let id = self.id.take().expect("event cleanup already ran");
        run(&["delete", &id, "--force", "--json"])
    }
}

impl Drop for EventCleanup {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = run(&["delete", &id, "--force", "--json"]);
        }
    }
}

struct ReminderCleanup {
    id: Option<String>,
}

impl ReminderCleanup {
    fn delete(&mut self) -> Output {
        let id = self.id.take().expect("reminder cleanup already ran");
        run(&["reminders", "delete", &id, "--force", "--json"])
    }
}

impl Drop for ReminderCleanup {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = run(&["reminders", "delete", &id, "--force", "--json"]);
        }
    }
}

#[test]
#[ignore = "manual macOS EventKit check; reads local permission and default-calendar state"]
fn permission_and_default_calendar_are_parseable() {
    let doctor = run(&["doctor", "--json"]);
    assert!(doctor.status.success());
    let doctor = json(&doctor);
    assert_eq!(doctor["type"], "doctor");
    assert_eq!(doctor["doctor"]["authorization"], "FullAccess");

    let default = run(&["default-calendar", "--json"]);
    assert!(default.status.success());
    let default = json(&default);
    assert_eq!(default["type"], "default_calendar");
    assert!(default["calendar"]["id"].is_string());
    assert_eq!(default["calendar"]["is_default_for_new_events"], true);
}

#[test]
#[ignore = "manual destructive test; requires explicit opt-in and an exact `icalctl Test` calendar"]
fn create_read_back_and_delete_on_explicit_test_calendar() {
    assert_eq!(
        std::env::var("ICALCTL_RUN_EVENTKIT_TESTS").as_deref(),
        Ok("1"),
        "set ICALCTL_RUN_EVENTKIT_TESTS=1 to acknowledge real Calendar writes"
    );
    let calendar_id = std::env::var("ICALCTL_TEST_CALENDAR_ID")
        .expect("set ICALCTL_TEST_CALENDAR_ID to the exact id of an `icalctl Test` calendar");

    let calendars = run(&["calendars", "--json"]);
    assert!(calendars.status.success());
    let calendars = json(&calendars);
    let calendar = calendars["calendars"]
        .as_array()
        .unwrap()
        .iter()
        .find(|calendar| calendar["id"] == calendar_id)
        .expect("ICALCTL_TEST_CALENDAR_ID was not found");
    assert_eq!(calendar["title"], TEST_CALENDAR_TITLE);
    assert_eq!(calendar["allows_modifications"], true);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let title = format!("icalctl manual integration {nonce}");
    let created = run(&[
        "add",
        &title,
        "--calendar-id",
        &calendar_id,
        "--start",
        "2099-12-30T09:00:00+00:00",
        "--end",
        "2099-12-30T10:00:00+00:00",
        "--time-zone",
        "Europe/Berlin",
        "--alarm-minutes-before",
        "5",
        "--if-exists",
        "error",
        "--json",
    ]);
    assert!(
        created.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created = json(&created);
    let id = created["event"]["id"]
        .as_str()
        .expect("created event id is missing")
        .to_string();
    let mut cleanup = EventCleanup {
        id: Some(id.clone()),
    };

    assert_eq!(created["event"]["calendar_id"], calendar_id);
    assert_eq!(created["event"]["write_action"], "created");
    assert_eq!(created["event"]["start_utc"], "2099-12-30T09:00:00+00:00");
    assert_eq!(created["event"]["duration_seconds"], 3600);
    assert_eq!(created["event"]["alarm_count"], 1);

    let shown = run(&["show", &id, "--json"]);
    assert!(shown.status.success());
    let shown = json(&shown);
    assert_eq!(shown["event"]["id"], id);
    assert_eq!(shown["event"]["calendar_id"], calendar_id);
    assert_eq!(shown["event"]["timezone"], "Europe/Berlin");
    assert_eq!(shown["event"]["alarm_count"], 1);

    let deleted = cleanup.delete();
    assert!(
        deleted.status.success(),
        "cleanup delete failed: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );
}

#[test]
#[ignore = "manual destructive test; requires explicit opt-in and an exact `icalctl Test` reminder list"]
fn create_read_back_clear_and_delete_advanced_reminder() {
    assert_eq!(
        std::env::var("ICALCTL_RUN_REMINDER_EVENTKIT_TESTS").as_deref(),
        Ok("1"),
        "set ICALCTL_RUN_REMINDER_EVENTKIT_TESTS=1 to acknowledge real Reminders writes"
    );
    let list_id = std::env::var("ICALCTL_TEST_REMINDER_LIST_ID")
        .expect("set ICALCTL_TEST_REMINDER_LIST_ID to an exact `icalctl Test` reminder-list id");
    let lists = run(&["reminders", "lists", "--json"]);
    assert!(lists.status.success());
    let lists = json(&lists);
    let list = lists["lists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|list| list["id"] == list_id)
        .expect("ICALCTL_TEST_REMINDER_LIST_ID was not found");
    assert_eq!(list["title"], TEST_REMINDER_LIST_TITLE);
    assert_eq!(list["allows_modifications"], true);

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let title = format!("icalctl advanced reminder integration {nonce}");
    let created = run(&[
        "reminders",
        "add",
        &title,
        "--list-id",
        &list_id,
        "--due",
        "2099-12-30T09:00:00+00:00",
        "--notify-at",
        "2099-12-30T08:00:00.000900+00:00",
        "--geofence-title",
        "Helsinki",
        "--geofence-latitude",
        "60.1699",
        "--geofence-longitude",
        "24.9384",
        "--geofence-radius-meters",
        "150",
        "--geofence-proximity",
        "arrive",
        "--repeat",
        "weekly",
        "--repeat-count",
        "3",
        "--if-exists",
        "error",
        "--json",
    ]);
    assert!(
        created.status.success(),
        "reminder add failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let created = json(&created);
    let id = created["reminder"]["id"]
        .as_str()
        .expect("created reminder id is missing")
        .to_string();
    let mut cleanup = ReminderCleanup {
        id: Some(id.clone()),
    };
    assert_eq!(created["reminder"]["list_id"], list_id);
    assert_eq!(created["reminder"]["alarm_count"], 2);
    assert_eq!(created["reminder"]["recurrence_count"], 1);

    let shown = run(&["reminders", "show", &id, "--json"]);
    assert!(shown.status.success());
    let shown = json(&shown);
    let alarms = shown["reminder"]["alarms"].as_array().unwrap();
    assert_eq!(alarms.len(), 2);
    let geofence = alarms
        .iter()
        .find(|alarm| alarm["proximity"] == "arrive")
        .expect("geofence alarm did not round-trip");
    assert_eq!(geofence["structured_location"]["latitude"], 60.1699);
    assert_eq!(geofence["structured_location"]["longitude"], 24.9384);
    let absolute = alarms
        .iter()
        .find(|alarm| alarm["proximity"] == "none")
        .and_then(|alarm| alarm["absolute_date"].as_str())
        .expect("absolute alarm did not round-trip");
    let expected = DateTime::parse_from_rfc3339("2099-12-30T08:00:00.000900+00:00").unwrap();
    let actual = DateTime::parse_from_rfc3339(absolute).unwrap();
    assert!(
        (actual.timestamp_nanos_opt().unwrap() - expected.timestamp_nanos_opt().unwrap()).abs()
            <= 1_000,
        "absolute alarm precision changed: expected={expected} actual={actual}"
    );
    assert_eq!(
        shown["reminder"]["recurrence_rules"][0]["end"]["occurrence_count"],
        3
    );

    let cleared = run(&[
        "reminders",
        "update",
        &id,
        "--clear-notifications",
        "--clear-recurrence",
        "--json",
    ]);
    assert!(
        cleared.status.success(),
        "reminder clear failed: {}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    let cleared = json(&cleared);
    assert_eq!(cleared["reminder"]["alarm_count"], 0);
    assert_eq!(cleared["reminder"]["recurrence_count"], 0);

    let shown = run(&["reminders", "show", &id, "--json"]);
    assert!(shown.status.success());
    let shown = json(&shown);
    assert_eq!(shown["reminder"]["alarm_count"], 0);
    assert_eq!(shown["reminder"]["recurrence_count"], 0);
    assert!(shown["reminder"]["alarms"].as_array().unwrap().is_empty());
    assert!(
        shown["reminder"]["recurrence_rules"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let deleted = cleanup.delete();
    assert!(deleted.status.success());
    assert_eq!(json(&deleted)["type"], "reminder_deleted");
}
