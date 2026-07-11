use chrono::DateTime;
use serde_json::Value;
use std::cell::RefCell;
use std::io;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const ICALCTL: &str = env!("CARGO_BIN_EXE_icalctl");
const TEST_CALENDAR_TITLE: &str = "icalctl Test";
const TEST_REMINDER_LIST_TITLE: &str = "icalctl Test";

fn run(args: &[&str]) -> Output {
    try_run(args).expect("failed to run icalctl test binary")
}

fn try_run(args: &[&str]) -> io::Result<Output> {
    Command::new(ICALCTL).args(args).output()
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

struct RecurringEventCleanup {
    title: String,
    calendar_id: String,
    from: &'static str,
    to: &'static str,
    fallback: RefCell<Option<(String, String)>>,
}

impl RecurringEventCleanup {
    fn try_occurrences(&self) -> Result<Vec<Value>, String> {
        let output = try_run(&[
            "search",
            &self.title,
            "--from",
            self.from,
            "--to",
            self.to,
            "--calendar-id",
            &self.calendar_id,
            "--json",
        ])
        .map_err(|error| format!("failed to run recurring event search: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "recurring event search failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("invalid recurring search JSON: {error}"))?;
        value["events"]
            .as_array()
            .cloned()
            .ok_or_else(|| "recurring search JSON has no events array".to_string())
    }

    fn occurrences(&self) -> Vec<Value> {
        self.try_occurrences().unwrap()
    }

    fn try_delete_all(&self) -> Result<(), String> {
        let mut deleted_any = false;
        for _ in 0..16 {
            let occurrences = match self.try_occurrences() {
                Ok(occurrences) => occurrences,
                Err(error) => {
                    return self.try_delete_fallback().map_err(|fallback| {
                        format!("{error}; fallback cleanup failed: {fallback}")
                    });
                }
            };
            let Some(event) = occurrences.into_iter().next() else {
                if deleted_any {
                    self.fallback.borrow_mut().take();
                    return Ok(());
                }
                return self.try_delete_fallback();
            };
            let id = event["id"]
                .as_str()
                .ok_or_else(|| "cleanup event has no id".to_string())?;
            let start = event["start"]
                .as_str()
                .ok_or_else(|| "cleanup event has no start".to_string())?;
            let deleted = try_run(&[
                "delete",
                id,
                "--occurrence-start",
                start,
                "--scope",
                "occurrence",
                "--force",
                "--json",
            ])
            .map_err(|error| format!("failed to run recurring cleanup delete: {error}"))?;
            if !deleted.status.success() {
                return Err(format!(
                    "recurring cleanup delete failed: {}",
                    String::from_utf8_lossy(&deleted.stderr)
                ));
            }
            deleted_any = true;
        }
        Err("recurring cleanup exceeded its occurrence limit".to_string())
    }

    fn try_delete_fallback(&self) -> Result<(), String> {
        let Some((fallback_id, fallback_start)) = self.fallback.borrow().clone() else {
            return Ok(());
        };
        let deleted = try_run(&[
            "delete",
            &fallback_id,
            "--occurrence-start",
            &fallback_start,
            "--scope",
            "future",
            "--force",
            "--json",
        ])
        .map_err(|error| format!("failed to run fallback recurring delete: {error}"))?;
        if !deleted.status.success() {
            return Err(format!(
                "fallback recurring delete failed: {}",
                String::from_utf8_lossy(&deleted.stderr)
            ));
        }
        self.fallback.borrow_mut().take();
        Ok(())
    }

    fn delete_all(&self) {
        self.try_delete_all().unwrap();
    }
}

impl Drop for RecurringEventCleanup {
    fn drop(&mut self) {
        let _ = self.try_delete_all();
    }
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
#[ignore = "manual destructive recurrence test; requires explicit opt-in and an exact `icalctl Test` calendar"]
fn recurring_dst_all_day_and_scoped_mutations_on_explicit_test_calendar() {
    assert_eq!(
        std::env::var("ICALCTL_RUN_EVENTKIT_TESTS").as_deref(),
        Ok("1"),
        "set ICALCTL_RUN_EVENTKIT_TESTS=1 to acknowledge real Calendar writes"
    );
    let calendar_id = std::env::var("ICALCTL_TEST_CALENDAR_ID")
        .expect("set ICALCTL_TEST_CALENDAR_ID to the exact id of an `icalctl Test` calendar");
    let calendars = json(&run(&["calendars", "--json"]));
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
    let timed_title = format!("icalctl recurring DST integration {nonce}");
    let created = run(&[
        "add",
        &timed_title,
        "--calendar-id",
        &calendar_id,
        "--start",
        "2030-03-24T09:00",
        "--end",
        "2030-03-24T10:00",
        "--time-zone",
        "Europe/Berlin",
        "--repeat",
        "weekly",
        "--repeat-count",
        "4",
        "--if-exists",
        "error",
        "--json",
    ]);
    assert!(
        created.status.success(),
        "recurring add failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let timed_cleanup = RecurringEventCleanup {
        title: timed_title,
        calendar_id: calendar_id.clone(),
        from: "2030-03-20",
        to: "2030-04-20",
        fallback: RefCell::new(None),
    };
    let created = json(&created);
    *timed_cleanup.fallback.borrow_mut() = Some((
        created["event"]["id"].as_str().unwrap().to_string(),
        created["event"]["start"].as_str().unwrap().to_string(),
    ));
    let occurrences = timed_cleanup.occurrences();
    assert_eq!(occurrences.len(), 4);
    assert!(
        occurrences[0]["start_in_event_time_zone"]
            .as_str()
            .unwrap()
            .starts_with("2030-03-24T09:00:00+01:00")
    );
    assert!(
        occurrences[1]["start_in_event_time_zone"]
            .as_str()
            .unwrap()
            .starts_with("2030-03-31T09:00:00+02:00")
    );

    let second_id = occurrences[1]["id"].as_str().unwrap();
    let second_start = occurrences[1]["start"].as_str().unwrap();
    let occurrence_update = run(&[
        "update",
        second_id,
        "--occurrence-start",
        second_start,
        "--scope",
        "occurrence",
        "--notes",
        "only this occurrence",
        "--json",
    ]);
    assert!(occurrence_update.status.success());
    let occurrence_update = json(&occurrence_update);
    assert_eq!(occurrence_update["event"]["write_scope"], "occurrence");
    assert_eq!(occurrence_update["event"]["is_detached"], true);

    let occurrences = timed_cleanup.occurrences();
    assert_eq!(occurrences[1]["id"], occurrence_update["event"]["id"]);
    assert_eq!(occurrences[1]["start"], occurrence_update["event"]["start"]);
    assert_eq!(occurrences[0]["notes"], Value::Null);
    assert_eq!(occurrences[1]["notes"], "only this occurrence");
    assert_eq!(occurrences[2]["notes"], Value::Null);
    let third_id = occurrences[2]["id"].as_str().unwrap();
    let third_start = occurrences[2]["start"].as_str().unwrap();
    let future_update = run(&[
        "update",
        third_id,
        "--occurrence-start",
        third_start,
        "--scope",
        "future",
        "--location",
        "future only",
        "--json",
    ]);
    assert!(future_update.status.success());
    let future_update = json(&future_update);
    assert_eq!(future_update["event"]["write_scope"], "future");

    let occurrences = timed_cleanup.occurrences();
    assert_eq!(occurrences[2]["id"], future_update["event"]["id"]);
    assert_eq!(occurrences[2]["start"], future_update["event"]["start"]);
    assert_eq!(occurrences[0]["location"], Value::Null);
    assert_eq!(occurrences[1]["location"], Value::Null);
    assert_eq!(occurrences[2]["location"], "future only");
    assert_eq!(occurrences[3]["location"], "future only");

    let detached_delete = run(&[
        "delete",
        occurrences[1]["id"].as_str().unwrap(),
        "--occurrence-start",
        occurrences[1]["start"].as_str().unwrap(),
        "--scope",
        "occurrence",
        "--force",
        "--json",
    ]);
    assert!(detached_delete.status.success());
    let occurrences = timed_cleanup.occurrences();
    assert_eq!(occurrences.len(), 3);
    let future_delete = run(&[
        "delete",
        occurrences[1]["id"].as_str().unwrap(),
        "--occurrence-start",
        occurrences[1]["start"].as_str().unwrap(),
        "--scope",
        "future",
        "--force",
        "--json",
    ]);
    assert!(future_delete.status.success());
    assert_eq!(timed_cleanup.occurrences().len(), 1);
    timed_cleanup.delete_all();

    let all_day_title = format!("icalctl recurring all-day integration {nonce}");
    let all_day_created = run(&[
        "add",
        &all_day_title,
        "--calendar-id",
        &calendar_id,
        "--start",
        "2030-03-30",
        "--end",
        "2030-03-30",
        "--all-day",
        "--time-zone",
        "Europe/Berlin",
        "--repeat",
        "daily",
        "--repeat-count",
        "3",
        "--if-exists",
        "error",
        "--json",
    ]);
    assert!(
        all_day_created.status.success(),
        "all-day recurring add failed: {}",
        String::from_utf8_lossy(&all_day_created.stderr)
    );
    let all_day_cleanup = RecurringEventCleanup {
        title: all_day_title,
        calendar_id,
        from: "2030-03-29",
        to: "2030-04-03",
        fallback: RefCell::new(None),
    };
    let all_day_created = json(&all_day_created);
    *all_day_cleanup.fallback.borrow_mut() = Some((
        all_day_created["event"]["id"].as_str().unwrap().to_string(),
        all_day_created["event"]["start"]
            .as_str()
            .unwrap()
            .to_string(),
    ));
    let all_day = all_day_cleanup.occurrences();
    assert_eq!(all_day.len(), 3);
    for event in &all_day {
        assert_eq!(event["all_day"], true);
    }
    assert!(
        all_day[0]["start_in_event_time_zone"]
            .as_str()
            .unwrap()
            .starts_with("2030-03-30T00:00:00+01:00")
    );
    assert!(
        all_day[1]["start_in_event_time_zone"]
            .as_str()
            .unwrap()
            .starts_with("2030-03-31T00:00:00+01:00")
    );
    assert!(
        all_day[2]["start_in_event_time_zone"]
            .as_str()
            .unwrap()
            .starts_with("2030-04-01T00:00:00+02:00")
    );
    all_day_cleanup.delete_all();
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

    let completed = run(&[
        "reminders",
        "complete",
        &id,
        "--completed-at",
        "2099-12-29T12:34:56.789123+00:00",
        "--json",
    ]);
    assert!(completed.status.success());
    assert_eq!(json(&completed)["reminder"]["completed"], true);
    let uncompleted = run(&["reminders", "uncomplete", &id, "--json"]);
    assert!(uncompleted.status.success());
    assert_eq!(json(&uncompleted)["reminder"]["completed"], false);

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
