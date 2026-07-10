use serde_json::Value;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const ICALCTL: &str = env!("CARGO_BIN_EXE_icalctl");
const TEST_CALENDAR_TITLE: &str = "icalctl Test";

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
