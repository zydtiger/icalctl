use super::*;

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn append_help(command: &mut clap::Command, path: &str, output: &mut Vec<u8>) {
    output.extend_from_slice(path.as_bytes());
    output.push(b'\n');
    command.write_long_help(&mut *output).unwrap();
    output.extend_from_slice(b"\n---\n");

    for child in command.get_subcommands_mut() {
        let child_path = format!("{path} {}", child.get_name());
        append_help(child, &child_path, output);
    }
}

fn command_paths(command: &clap::Command, prefix: &str, output: &mut Vec<String>) {
    for child in command.get_subcommands() {
        let path = format!("{prefix} {}", child.get_name());
        output.push(path.clone());
        command_paths(child, &path, output);
    }
}

fn completion_hash(shell: Shell) -> u64 {
    let mut command = Cli::command();
    let name = command.get_name().to_string();
    let mut output = Vec::new();
    generate(shell, &mut command, name, &mut output);
    stable_hash(&output)
}

fn assert_contract_error(base: &[&str], extra: &[&str], kind: clap::error::ErrorKind) {
    let error = Cli::try_parse_from(base.iter().chain(extra).copied()).unwrap_err();
    assert_eq!(error.kind(), kind, "argv: {base:?} + {extra:?}");
}

#[test]
fn command_tree_is_stable() {
    let mut paths = Vec::new();
    command_paths(&Cli::command(), "icalctl", &mut paths);

    assert_eq!(
        paths,
        [
            "icalctl config",
            "icalctl config path",
            "icalctl config init",
            "icalctl config show",
            "icalctl config validate",
            "icalctl config edit",
            "icalctl config get",
            "icalctl config set",
            "icalctl config unset",
            "icalctl version",
            "icalctl status",
            "icalctl doctor",
            "icalctl calendars",
            "icalctl default-calendar",
            "icalctl list",
            "icalctl today",
            "icalctl upcoming",
            "icalctl show",
            "icalctl search",
            "icalctl add",
            "icalctl update",
            "icalctl batch",
            "icalctl batch add",
            "icalctl travel",
            "icalctl travel serve",
            "icalctl travel flight",
            "icalctl reminders",
            "icalctl reminders status",
            "icalctl reminders lists",
            "icalctl reminders default-list",
            "icalctl reminders list",
            "icalctl reminders search",
            "icalctl reminders show",
            "icalctl reminders add",
            "icalctl reminders batch",
            "icalctl reminders batch add",
            "icalctl reminders update",
            "icalctl reminders complete",
            "icalctl reminders uncomplete",
            "icalctl reminders delete",
            "icalctl delete",
            "icalctl completions",
        ]
    );
}

#[test]
fn long_help_contract_is_stable() {
    let mut command = Cli::command();
    let mut output = Vec::new();
    append_help(&mut command, "icalctl", &mut output);

    assert_eq!(stable_hash(&output), 17_482_121_679_375_552_753);
}

#[test]
fn generated_shell_completions_are_stable() {
    assert_eq!(
        [
            completion_hash(Shell::Bash),
            completion_hash(Shell::Elvish),
            completion_hash(Shell::Fish),
            completion_hash(Shell::PowerShell),
            completion_hash(Shell::Zsh),
        ],
        [
            18_329_766_573_192_339_178,
            6_548_920_715_031_450_731,
            8_934_612_081_613_975_269,
            17_633_123_951_638_291_442,
            10_859_355_139_673_504_805,
        ]
    );
}

#[test]
fn argument_relationship_contract_is_stable() {
    use clap::error::ErrorKind::{ArgumentConflict, MissingRequiredArgument};

    const TODAY: &[&str] = &["icalctl", "today"];
    const EVENT_ADD: &[&str] = &[
        "icalctl",
        "add",
        "Trip",
        "--start",
        "2026-07-14T10:00+02:00",
        "--end",
        "2026-07-14T11:00+02:00",
    ];
    const EVENT_UPDATE: &[&str] = &["icalctl", "update", "event-id"];
    const REMINDER_LIST: &[&str] = &["icalctl", "reminders", "list"];
    const REMINDER_ADD: &[&str] = &["icalctl", "reminders", "add", "Task"];
    const REMINDER_UPDATE: &[&str] = &["icalctl", "reminders", "update", "reminder-id"];

    // Read and write Calendar selector requirements and conflicts.
    assert_contract_error(
        TODAY,
        &["--calendar-source", "iCloud"],
        MissingRequiredArgument,
    );
    assert_contract_error(TODAY, &["--source-id", "source"], MissingRequiredArgument);
    assert_contract_error(
        TODAY,
        &[
            "--calendar",
            "Trips",
            "--calendar-source",
            "iCloud",
            "--source-id",
            "source",
        ],
        ArgumentConflict,
    );
    assert_contract_error(
        EVENT_ADD,
        &["--calendar-source", "iCloud"],
        MissingRequiredArgument,
    );
    assert_contract_error(
        EVENT_ADD,
        &["--source-id", "source"],
        MissingRequiredArgument,
    );
    assert_contract_error(
        EVENT_ADD,
        &["--calendar-id", "id", "--calendar", "Trips"],
        ArgumentConflict,
    );
    assert_contract_error(
        EVENT_ADD,
        &[
            "--calendar-id",
            "id",
            "--calendar",
            "Trips",
            "--calendar-source",
            "iCloud",
        ],
        ArgumentConflict,
    );
    assert_contract_error(
        EVENT_ADD,
        &[
            "--calendar-id",
            "id",
            "--calendar",
            "Trips",
            "--source-id",
            "source",
        ],
        ArgumentConflict,
    );
    assert_contract_error(
        EVENT_ADD,
        &[
            "--calendar",
            "Trips",
            "--calendar-source",
            "iCloud",
            "--source-id",
            "source",
        ],
        ArgumentConflict,
    );

    // Read and write Reminder-list selector requirements and conflicts.
    assert_contract_error(
        REMINDER_LIST,
        &["--list-source", "iCloud"],
        MissingRequiredArgument,
    );
    assert_contract_error(
        REMINDER_LIST,
        &["--source-id", "source"],
        MissingRequiredArgument,
    );
    assert_contract_error(
        REMINDER_LIST,
        &[
            "--list",
            "Inbox",
            "--list-source",
            "iCloud",
            "--source-id",
            "source",
        ],
        ArgumentConflict,
    );
    assert_contract_error(
        REMINDER_ADD,
        &["--list-source", "iCloud"],
        MissingRequiredArgument,
    );
    assert_contract_error(
        REMINDER_ADD,
        &["--source-id", "source"],
        MissingRequiredArgument,
    );
    assert_contract_error(
        REMINDER_ADD,
        &["--list-id", "id", "--list", "Inbox"],
        ArgumentConflict,
    );
    assert_contract_error(
        REMINDER_ADD,
        &[
            "--list-id",
            "id",
            "--list",
            "Inbox",
            "--list-source",
            "iCloud",
        ],
        ArgumentConflict,
    );
    assert_contract_error(
        REMINDER_ADD,
        &[
            "--list-id",
            "id",
            "--list",
            "Inbox",
            "--source-id",
            "source",
        ],
        ArgumentConflict,
    );
    assert_contract_error(
        REMINDER_ADD,
        &[
            "--list",
            "Inbox",
            "--list-source",
            "iCloud",
            "--source-id",
            "source",
        ],
        ArgumentConflict,
    );

    // Reminder schedule relationship graph.
    for missing in [
        "--geofence-latitude",
        "--geofence-longitude",
        "--geofence-radius-meters",
        "--geofence-proximity",
    ] {
        let complete = [
            "--geofence-title",
            "Office",
            "--geofence-latitude",
            "59.9",
            "--geofence-longitude",
            "10.7",
            "--geofence-radius-meters",
            "100",
            "--geofence-proximity",
            "arrive",
        ];
        let mut incomplete = Vec::new();
        let mut skip_value = false;
        for value in complete {
            if value == missing {
                skip_value = true;
            } else if skip_value {
                skip_value = false;
            } else {
                incomplete.push(value);
            }
        }
        assert_contract_error(REMINDER_ADD, &incomplete, MissingRequiredArgument);
    }
    for component in [
        &["--geofence-latitude", "59.9"][..],
        &["--geofence-longitude", "10.7"][..],
        &["--geofence-radius-meters", "100"][..],
        &["--geofence-proximity", "arrive"][..],
    ] {
        assert_contract_error(REMINDER_ADD, component, MissingRequiredArgument);
    }
    for recurrence in [
        &["--repeat-interval", "2"][..],
        &["--repeat-count", "3"][..],
        &["--repeat-until", "2026-08-01T10:00+02:00"][..],
    ] {
        assert_contract_error(REMINDER_ADD, recurrence, MissingRequiredArgument);
    }
    assert_contract_error(
        REMINDER_ADD,
        &[
            "--repeat",
            "daily",
            "--repeat-count",
            "3",
            "--repeat-until",
            "2026-08-01T10:00+02:00",
        ],
        ArgumentConflict,
    );

    // Event recurrence requirements and termination conflict.
    for recurrence in [
        &["--repeat-interval", "2"][..],
        &["--repeat-weekday", "monday"][..],
        &["--repeat-month-day", "14"][..],
        &["--repeat-count", "3"][..],
        &["--repeat-until", "2026-08-01T10:00+02:00"][..],
    ] {
        assert_contract_error(EVENT_ADD, recurrence, MissingRequiredArgument);
    }
    assert_contract_error(
        EVENT_ADD,
        &[
            "--repeat",
            "daily",
            "--repeat-count",
            "3",
            "--repeat-until",
            "2026-08-01T10:00+02:00",
        ],
        ArgumentConflict,
    );

    // Event add required-unless-json and mutation conflicts.
    assert_contract_error(
        &[
            "icalctl",
            "add",
            "--start",
            "2026-07-14",
            "--end",
            "2026-07-15",
        ],
        &[],
        MissingRequiredArgument,
    );
    assert_contract_error(
        &["icalctl", "add", "Trip", "--end", "2026-07-15"],
        &[],
        MissingRequiredArgument,
    );
    assert_contract_error(
        &["icalctl", "add", "Trip", "--start", "2026-07-14"],
        &[],
        MissingRequiredArgument,
    );
    assert_contract_error(
        EVENT_ADD,
        &["--notes", "text", "--notes-file", "notes.txt"],
        ArgumentConflict,
    );
    for conflict in [
        &["--notes", "text", "--clear-notes"][..],
        &["--location", "Office", "--clear-location"][..],
        &["--url", "https://example.com", "--clear-url"][..],
        &["--all-day", "--timed"][..],
        &["--time-zone", "Europe/Oslo", "--clear-time-zone"][..],
    ] {
        assert_contract_error(EVENT_UPDATE, conflict, ArgumentConflict);
    }

    // Configuration, Reminder mutation, and travel-flight conflicts.
    assert_contract_error(
        &[
            "icalctl",
            "config",
            "set",
            "calendar.default_calendar_id",
            "id",
        ],
        &["--stdin"],
        ArgumentConflict,
    );
    assert_contract_error(
        REMINDER_ADD,
        &["--notes", "text", "--notes-file", "notes.txt"],
        ArgumentConflict,
    );
    for conflict in [
        &["--parent-id", "parent", "--clear-parent"][..],
        &["--due", "2026-07-14", "--clear-due"][..],
        &["--start", "2026-07-14", "--clear-start"][..],
        &["--time-zone", "Europe/Oslo", "--clear-time-zone"][..],
        &["--notes", "text", "--notes-file", "notes.txt"][..],
        &["--notes", "text", "--clear-notes"][..],
        &["--notes-file", "notes.txt", "--clear-notes"][..],
        &["--url", "https://example.com", "--clear-url"][..],
        &["--location", "Office", "--clear-location"][..],
        &["--clear-notifications", "--notify-at-due"][..],
        &["--clear-notifications", "--notify-minutes-before", "10"][..],
        &[
            "--clear-notifications",
            "--notify-at",
            "2026-07-14T10:00+02:00",
        ][..],
        &[
            "--clear-notifications",
            "--geofence-title",
            "Office",
            "--geofence-latitude",
            "59.9",
            "--geofence-longitude",
            "10.7",
            "--geofence-radius-meters",
            "100",
            "--geofence-proximity",
            "arrive",
        ][..],
        &["--clear-recurrence", "--repeat", "daily"][..],
    ] {
        assert_contract_error(REMINDER_UPDATE, conflict, ArgumentConflict);
    }
    assert_contract_error(
        &[
            "icalctl",
            "travel",
            "flight",
            "DY627",
            "--from",
            "BGO",
            "--to",
            "OSL",
            "--departure",
            "2026-07-14T10:00+02:00",
            "--arrival",
            "2026-07-14T11:00+02:00",
        ],
        &["--notes", "text", "--notes-file", "notes.txt"],
        ArgumentConflict,
    );
}

#[test]
fn version_command_parses() {
    let cli = Cli::try_parse_from(["icalctl", "version"]).unwrap();

    assert!(matches!(cli.command, Command::Version));
}

#[test]
fn config_commands_are_small_and_generic() {
    assert!(matches!(
        Cli::try_parse_from(["icalctl", "config", "show"])
            .unwrap()
            .command,
        Command::Config {
            command: ConfigCommand::Show
        }
    ));
    assert!(matches!(
        Cli::try_parse_from([
            "icalctl",
            "config",
            "set",
            "calendar.default_calendar_id",
            "CAL-1"
        ])
        .unwrap()
        .command,
        Command::Config {
            command: ConfigCommand::Set {
                key,
                value: Some(value),
                stdin: false,
            }
        } if key == "calendar.default_calendar_id" && value == "CAL-1"
    ));
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "config",
            "set",
            "flightaware.api_key",
            "secret",
            "--stdin"
        ])
        .is_err()
    );
}

#[test]
fn standard_version_flag_uses_cargo_package_version() {
    let error = Cli::try_parse_from(["icalctl", "--version"]).unwrap_err();

    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
    assert!(error.to_string().contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn source_qualifier_requires_calendar_title() {
    let result = Cli::try_parse_from(["icalctl", "today", "--calendar-source", "iCloud"]);

    assert!(result.is_err());
}

#[test]
fn default_calendar_command_parses() {
    let cli = Cli::try_parse_from(["icalctl", "default-calendar"]).unwrap();

    assert!(matches!(cli.command, Command::DefaultCalendar));
}

#[test]
fn doctor_command_parses() {
    let cli = Cli::try_parse_from(["icalctl", "doctor"]).unwrap();

    assert!(matches!(cli.command, Command::Doctor));
}

#[test]
fn calendars_accepts_source_and_writable_filters() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "calendars",
        "--source",
        "iCloud",
        "--writable-only",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Calendars {
            source: Some(source),
            writable_only: true,
        } if source == "iCloud"
    ));
}

#[test]
fn read_commands_accept_multiple_exact_calendar_ids() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "list",
        "--from",
        "2026-07-10",
        "--to",
        "2026-07-10",
        "--calendar-id",
        "A",
        "--calendar-id",
        "B",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::List {
            calendar_selector: ReadCalendarSelectorArgs { calendar_ids, .. },
            ..
        } if calendar_ids == ["A", "B"]
    ));
}

#[test]
fn write_calendar_id_conflicts_with_title() {
    let result = Cli::try_parse_from([
        "icalctl",
        "add",
        "Meeting",
        "--start",
        "2026-07-10T09:00",
        "--end",
        "2026-07-10T10:00",
        "--calendar",
        "Calendar",
        "--calendar-id",
        "ABC",
    ]);

    assert!(result.is_err());
}

#[test]
fn add_and_update_accept_dry_run() {
    let add = Cli::try_parse_from([
        "icalctl",
        "add",
        "Meeting",
        "--start",
        "2026-07-10T09:00",
        "--end",
        "2026-07-10T10:00",
        "--dry-run",
    ]);
    let update = Cli::try_parse_from([
        "icalctl",
        "update",
        "EVENT-ID",
        "--title",
        "Meeting",
        "--dry-run",
    ]);

    assert!(add.is_ok());
    assert!(update.is_ok());
}

#[test]
fn add_accepts_duplicate_policy_and_window() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "add",
        "Meeting",
        "--start",
        "2026-07-10T09:00",
        "--end",
        "2026-07-10T10:00",
        "--if-exists",
        "skip",
        "--duplicate-window-seconds",
        "30",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Add {
            if_exists: IfExistsArg::Skip,
            duplicate_window_seconds: 30,
            ..
        }
    ));
}

#[test]
fn add_accepts_notes_file_or_complete_json_file() {
    let notes = Cli::try_parse_from([
        "icalctl",
        "add",
        "Meeting",
        "--start",
        "2026-07-10T09:00",
        "--end",
        "2026-07-10T10:00",
        "--notes-file",
        "notes.txt",
    ]);
    let json = Cli::try_parse_from(["icalctl", "add", "--json-file", "event.json", "--dry-run"]);

    assert!(notes.is_ok());
    assert!(json.is_ok());
}

#[test]
fn add_rejects_notes_and_notes_file_together() {
    let result = Cli::try_parse_from([
        "icalctl",
        "add",
        "Meeting",
        "--start",
        "2026-07-10T09:00",
        "--end",
        "2026-07-10T10:00",
        "--notes",
        "inline",
        "--notes-file",
        "notes.txt",
    ]);

    assert!(result.is_err());
}

#[test]
fn add_duplicate_policy_defaults_to_error_and_exact_times() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "add",
        "Meeting",
        "--start",
        "2026-07-10T09:00",
        "--end",
        "2026-07-10T10:00",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Add {
            if_exists: IfExistsArg::Error,
            duplicate_window_seconds: 0,
            ..
        }
    ));
}

#[test]
fn add_and_update_accept_event_time_zone() {
    let add = Cli::try_parse_from([
        "icalctl",
        "add",
        "Flight",
        "--start",
        "2026-07-12T15:55:00+03:00",
        "--end",
        "2026-07-12T15:55:00+02:00",
        "--time-zone",
        "Europe/Berlin",
    ]);
    let update = Cli::try_parse_from(["icalctl", "update", "EVENT-ID", "--clear-time-zone"]);

    assert!(add.is_ok());
    assert!(update.is_ok());
}

#[test]
fn update_rejects_set_and_clear_time_zone_together() {
    let result = Cli::try_parse_from([
        "icalctl",
        "update",
        "EVENT-ID",
        "--time-zone",
        "Europe/Berlin",
        "--clear-time-zone",
    ]);

    assert!(result.is_err());
}

#[test]
fn batch_add_parses_safety_flags() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "batch",
        "add",
        "--file",
        "events.json",
        "--if-exists",
        "skip",
        "--dry-run",
        "--continue-on-error",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Batch {
            command: BatchCommand::Add {
                if_exists: IfExistsArg::Skip,
                dry_run: true,
                continue_on_error: true,
                ..
            }
        }
    ));
}

#[test]
fn travel_flight_forwards_add_options() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "travel",
        "flight",
        "ho1607",
        "--from",
        "pvg",
        "--to",
        "hel",
        "--departure",
        "2026-07-11T09:25:00+08:00",
        "--arrival",
        "2026-07-11T14:00:00+03:00",
        "--calendar",
        "Travel",
        "--calendar-source",
        "iCloud",
        "--notes",
        "Booking confirmed",
        "--url",
        "https://example.com/flight",
        "--availability",
        "free",
        "--alarm-minutes-before",
        "30",
        "--if-exists",
        "skip",
        "--duplicate-window-seconds",
        "60",
        "--dry-run",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Travel {
            command: TravelCommand::Flight {
                flight_number,
                from_airport,
                to_airport,
                calendar_selector: WriteCalendarSelectorArgs {
                    calendar: Some(calendar),
                    calendar_source: Some(source),
                    ..
                },
                notes: Some(notes),
                url: Some(url),
                availability: AvailabilityArg::Free,
                alarm_minutes_before,
                if_exists: IfExistsArg::Skip,
                duplicate_window_seconds: 60,
                dry_run: true,
                ..
            }
        } if flight_number == "ho1607"
            && from_airport == "pvg"
            && to_airport == "hel"
            && calendar == "Travel"
            && source == "iCloud"
            && notes == "Booking confirmed"
            && url == "https://example.com/flight"
            && alarm_minutes_before == [30]
    ));
}

#[test]
fn travel_flight_defaults_to_busy_with_no_alarms() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "travel",
        "flight",
        "HO1607",
        "--from",
        "PVG",
        "--to",
        "HEL",
        "--departure",
        "2026-07-11T09:25:00+08:00",
        "--arrival",
        "2026-07-11T14:00:00+03:00",
        "--notes-file",
        "flight-notes.txt",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Travel {
            command: TravelCommand::Flight {
                availability: AvailabilityArg::Busy,
                alarm_minutes_before,
                notes_file: Some(notes_file),
                ..
            }
        } if alarm_minutes_before.is_empty()
            && notes_file.to_str() == Some("flight-notes.txt")
    ));
}

#[test]
fn reminder_status_lists_default_list_and_show_parse() {
    assert!(matches!(
        Cli::try_parse_from(["icalctl", "reminders", "status"])
            .unwrap()
            .command,
        Command::Reminders {
            command: RemindersCommand::Status
        }
    ));
    assert!(matches!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "lists",
            "--source",
            "iCloud",
            "--writable-only"
        ])
        .unwrap()
        .command,
        Command::Reminders {
            command: RemindersCommand::Lists {
                source: Some(source),
                writable_only: true,
            }
        } if source == "iCloud"
    ));
    assert!(matches!(
        Cli::try_parse_from(["icalctl", "reminders", "default-list"])
            .unwrap()
            .command,
        Command::Reminders {
            command: RemindersCommand::DefaultList
        }
    ));
    assert!(matches!(
        Cli::try_parse_from(["icalctl", "reminders", "show", "2"])
            .unwrap()
            .command,
        Command::Reminders {
            command: RemindersCommand::Show { id }
        } if id == "2"
    ));
}

#[test]
fn reminder_list_defaults_to_incomplete_and_accepts_repeated_selectors() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "list",
        "--list-id",
        "A",
        "--list-id",
        "B",
        "--due-from",
        "2026-07-10",
        "--due-to",
        "2026-07-15",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::List {
                filters: ReminderReadFilterArgs {
                    state: ReminderStateArg::Incomplete,
                    list_selector: ReadReminderListSelectorArgs { list_ids, .. },
                    due_from: Some(from),
                    due_to: Some(to),
                }
            }
        } if list_ids == ["A", "B"] && from == "2026-07-10" && to == "2026-07-15"
    ));
}

#[test]
fn reminder_search_accepts_completed_and_source_qualified_title() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "search",
        "report",
        "--list",
        "Work",
        "--list-source",
        "Exchange",
        "--state",
        "completed",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Search {
                query,
                filters: ReminderReadFilterArgs {
                    state: ReminderStateArg::Completed,
                    list_selector: ReadReminderListSelectorArgs {
                        lists,
                        list_source: Some(source),
                        ..
                    },
                    ..
                }
            }
        } if query == "report" && lists == ["Work"] && source == "Exchange"
    ));
}

#[test]
fn reminder_source_qualifier_requires_list_title() {
    let result = Cli::try_parse_from(["icalctl", "reminders", "list", "--list-source", "iCloud"]);

    assert!(result.is_err());
}

#[test]
fn reminder_add_forwards_safe_creation_options() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "add",
        "Submit report",
        "--list-id",
        "LIST-1",
        "--due",
        "2026-07-15T14:30",
        "--start",
        "2026-07-15",
        "--time-zone",
        "Europe/Helsinki",
        "--notes",
        "Final version",
        "--url",
        "https://example.com/report",
        "--location",
        "Office",
        "--priority",
        "high",
        "--notify-at-due",
        "--notify-minutes-before",
        "30",
        "--if-exists",
        "skip",
        "--duplicate-window-seconds",
        "30",
        "--dry-run",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Add {
                title: Some(title),
                list_selector: WriteReminderListSelectorArgs {
                    list_id: Some(list_id),
                    ..
                },
                due: Some(due),
                start: Some(start),
                time_zone: Some(time_zone),
                notes: Some(notes),
                url: Some(url),
                location: Some(location),
                priority: Some(ReminderPriorityArg::High),
                notify_at_due: true,
                notify_minutes_before,
                if_exists: IfExistsArg::Skip,
                duplicate_window_seconds: 30,
                dry_run: true,
                ..
            }
        } if title == "Submit report"
            && list_id == "LIST-1"
            && due == "2026-07-15T14:30"
            && start == "2026-07-15"
            && time_zone == "Europe/Helsinki"
            && notes == "Final version"
            && url == "https://example.com/report"
            && location == "Office"
            && notify_minutes_before == [30]
    ));
}

#[test]
fn reminder_add_allows_undated_default_list_with_priority_enum() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "add",
        "Call dentist",
        "--priority",
        "high",
        "--notes-file",
        "notes.txt",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Add {
                list_selector: WriteReminderListSelectorArgs {
                    list: None,
                    list_id: None,
                    ..
                },
                due: None,
                priority: Some(ReminderPriorityArg::High),
                notes_file: Some(path),
                ..
            }
        } if path.to_str() == Some("notes.txt")
    ));
}

#[test]
fn reminder_add_rejects_conflicting_list_and_notes_inputs() {
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "Task",
            "--list",
            "Tasks",
            "--list-id",
            "A",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "Task",
            "--notes",
            "inline",
            "--notes-file",
            "notes.txt",
        ])
        .is_err()
    );
}

#[test]
fn reminder_update_parses_patch_clear_move_and_dry_run_flags() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "update",
        "2",
        "--title",
        "Revised task",
        "--list-id",
        "LIST-2",
        "--due",
        "2026-07-15T14:30",
        "--time-zone",
        "Europe/Helsinki",
        "--clear-start",
        "--clear-notes",
        "--clear-url",
        "--location",
        "Office",
        "--priority",
        "none",
        "--dry-run",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Update {
                id,
                title: Some(title),
                list_selector: WriteReminderListSelectorArgs {
                    list_id: Some(list_id),
                    ..
                },
                due: Some(due),
                time_zone: Some(time_zone),
                clear_start: true,
                clear_notes: true,
                clear_url: true,
                location: Some(location),
                priority: Some(ReminderPriorityArg::None),
                dry_run: true,
                ..
            }
        } if id == "2"
            && title == "Revised task"
            && list_id == "LIST-2"
            && due == "2026-07-15T14:30"
            && time_zone == "Europe/Helsinki"
            && location == "Office"
    ));
}

#[test]
fn reminder_lifecycle_commands_parse() {
    assert!(matches!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "complete",
            "ID",
            "--completed-at",
            "2026-07-11T14:00:00+03:00",
            "--dry-run",
        ])
        .unwrap()
        .command,
        Command::Reminders {
            command: RemindersCommand::Complete {
                id,
                completed_at: Some(completed_at),
                dry_run: true,
            }
        } if id == "ID" && completed_at == "2026-07-11T14:00:00+03:00"
    ));
    assert!(matches!(
        Cli::try_parse_from(["icalctl", "reminders", "uncomplete", "ID", "--dry-run"])
            .unwrap()
            .command,
        Command::Reminders {
            command: RemindersCommand::Uncomplete { id, dry_run: true }
        } if id == "ID"
    ));
    assert!(matches!(
        Cli::try_parse_from(["icalctl", "reminders", "delete", "ID", "--force"])
            .unwrap()
            .command,
        Command::Reminders {
            command: RemindersCommand::Delete { id, force: true }
        } if id == "ID"
    ));
}

#[test]
fn reminder_update_rejects_conflicting_set_and_clear_flags() {
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "update",
            "ID",
            "--due",
            "2026-07-15",
            "--clear-due",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "update",
            "ID",
            "--notes",
            "text",
            "--clear-notes",
        ])
        .is_err()
    );
}

#[test]
fn reminder_add_parses_absolute_geofence_and_recurrence_options() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "add",
        "Visit office",
        "--due",
        "2026-07-15T09:00:00+03:00",
        "--notify-at",
        "2026-07-15T07:00:00+03:00",
        "--geofence-title",
        "Office",
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
        "--repeat-interval",
        "2",
        "--repeat-count",
        "6",
        "--dry-run",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Add {
                schedule: ReminderAdvancedScheduleArgs {
                    notify_at,
                    geofence_title: Some(title),
                    geofence_proximity: Some(ReminderGeofenceProximityArg::Arrive),
                    repeat: Some(ReminderRepeatArg::Weekly),
                    repeat_interval: Some(2),
                    repeat_count: Some(6),
                    ..
                },
                dry_run: true,
                ..
            }
        } if notify_at == ["2026-07-15T07:00:00+03:00"] && title == "Office"
    ));
}

#[test]
fn reminder_update_parses_schedule_clear_flags_and_rejects_partial_geofence() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "update",
        "ID",
        "--clear-notifications",
        "--clear-recurrence",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Update {
                clear_notifications: true,
                clear_recurrence: true,
                dry_run: true,
                ..
            }
        }
    ));
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "add",
            "Task",
            "--geofence-title",
            "Office",
        ])
        .is_err()
    );
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "update",
            "ID",
            "--clear-notifications",
            "--notify-at",
            "2026-07-15T09:00:00+03:00",
        ])
        .is_err()
    );
}

#[test]
fn reminder_add_accepts_json_file_without_positional_title() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "add",
        "--json-file",
        "reminder.json",
        "--if-exists",
        "skip",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Add {
                title: None,
                json_file: Some(path),
                if_exists: IfExistsArg::Skip,
                dry_run: true,
                ..
            }
        } if path.to_str() == Some("reminder.json")
    ));
}

#[test]
fn reminder_batch_add_parses_safety_flags() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "batch",
        "add",
        "--file",
        "reminders.json",
        "--if-exists",
        "update",
        "--dry-run",
        "--continue-on-error",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Batch {
                command: ReminderBatchCommand::Add {
                    file,
                    if_exists: IfExistsArg::Update,
                    dry_run: true,
                    continue_on_error: true,
                }
            }
        } if file.to_str() == Some("reminders.json")
    ));
}

#[test]
fn event_show_accepts_an_exact_occurrence_start() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "show",
        "SERIES-ID",
        "--occurrence-start",
        "2026-07-20T09:00:00+03:00",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Show {
            id,
            occurrence_start: Some(start),
        } if id == "SERIES-ID" && start == "2026-07-20T09:00:00+03:00"
    ));
}

#[test]
fn event_update_parses_recurring_occurrence_scope() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "update",
        "SERIES-ID",
        "--occurrence-start",
        "2026-07-20T09:00:00+03:00",
        "--scope",
        "future",
        "--title",
        "Moved standup",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Update {
            id,
            occurrence_start: Some(start),
            scope: Some(EventScopeArg::Future),
            ..
        } if id == "SERIES-ID" && start == "2026-07-20T09:00:00+03:00"
    ));
}

#[test]
fn event_delete_parses_recurring_occurrence_scope() {
    let cli = Cli::try_parse_from(["icalctl", "delete", "3", "--scope", "occurrence", "--force"])
        .unwrap();
    assert!(matches!(
        cli.command,
        Command::Delete {
            id,
            occurrence_start: None,
            scope: Some(EventScopeArg::Occurrence),
            force: true,
        } if id == "3"
    ));
}

#[test]
fn event_add_parses_recurrence_creation_flags() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "add",
        "Standup",
        "--start",
        "2026-07-20T09:00:00+03:00",
        "--end",
        "2026-07-20T09:30:00+03:00",
        "--repeat",
        "weekly",
        "--repeat-interval",
        "2",
        "--repeat-weekday",
        "monday",
        "--repeat-weekday",
        "wednesday",
        "--repeat-count",
        "8",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Add {
            recurrence: EventRecurrenceArgs {
                repeat: Some(EventRepeatArg::Weekly),
                interval: Some(2),
                weekdays,
                count: Some(8),
                ..
            },
            dry_run: true,
            ..
        } if weekdays == [EventWeekdayArg::Monday, EventWeekdayArg::Wednesday]
    ));
}

#[test]
fn reminder_add_accepts_exact_parent_id() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "add",
        "Child task",
        "--parent-id",
        "PARENT-ID",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Command::Reminders {
            command: RemindersCommand::Add {
                parent_id: Some(parent_id),
                dry_run: true,
                ..
            }
        } if parent_id == "PARENT-ID"
    ));
}

#[test]
fn reminder_update_reparent_and_clear_parent_conflict() {
    let reparent = Cli::try_parse_from([
        "icalctl",
        "reminders",
        "update",
        "CHILD-ID",
        "--parent-id",
        "PARENT-ID",
        "--dry-run",
    ])
    .unwrap();
    assert!(matches!(
        reparent.command,
        Command::Reminders {
            command: RemindersCommand::Update {
                parent_id: Some(parent_id),
                clear_parent: false,
                ..
            }
        } if parent_id == "PARENT-ID"
    ));
    assert!(
        Cli::try_parse_from([
            "icalctl",
            "reminders",
            "update",
            "CHILD-ID",
            "--parent-id",
            "PARENT-ID",
            "--clear-parent",
        ])
        .is_err()
    );
}

#[test]
fn travel_serve_accepts_only_stable_calendar_ids() {
    let cli = Cli::try_parse_from([
        "icalctl",
        "travel",
        "serve",
        "--calendar-id",
        "CAL-1",
        "--calendar-id",
        "CAL-2",
    ])
    .unwrap();

    assert!(matches!(
        cli.command,
        Command::Travel {
            command: TravelCommand::Serve { calendar_ids }
        } if calendar_ids == ["CAL-1", "CAL-2"]
    ));
}
