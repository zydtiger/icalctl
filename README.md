# icalctl

`icalctl` is a macOS command-line interface for the local Apple Calendar and
Reminders stores. It talks to Apple's EventKit framework from Rust, so it works
with accounts already configured in Calendar.app and Reminders.app, including
iCloud, Google, Exchange, and local accounts.

The command is intended for fast terminal workflows and scripts. Human-readable
output is the default; pass `--json` to any command for compact JSON.

## Status

This is an early Apple Silicon macOS CLI. It currently supports:

- listing calendars
- listing, showing, and searching events
- creating, updating, and deleting events
- validating and importing idempotent JSON event batches
- compact JSON output
- cached row-number references for the most recent event list
- Reminders authorization, list discovery, list/search/show, safe dry-run
  creation, and a separate reminder row cache
- shell completion generation

It does not yet implement ICS import/export, recurrence editing UX, invite/RSVP
workflows, or Homebrew packaging.

## Requirements

- macOS
- Apple Calendar configured with the accounts you want to access
- Rust toolchain for local builds
- Calendar and Reminders permissions granted through their separate macOS
  privacy prompts as needed

The project currently targets Apple Silicon macOS (`aarch64-apple-darwin`).

## Install

From the project root:

```sh
cargo install --path .
```

`cargo install --path .` builds the release profile and installs the binary to
Cargo's normal bin directory, usually:

```text
~/.cargo/bin/icalctl
```

Make sure `~/.cargo/bin` is in `PATH`.

Verify the installed build:

```sh
icalctl version
icalctl version --json
icalctl --version
```

`Cargo.toml`'s `[package].version` is the single semantic-version source. The
build also embeds the current Git commit when built from a checkout, plus the
target triple and Cargo profile. Source archives without Git metadata report an
unknown commit unless `ICALCTL_GIT_COMMIT` is set while building. To release a
new version, update `Cargo.toml`, run the full checks, and reinstall with
`cargo install --path . --force`.

### Install the agent skill

The recommended way to install the bundled `icalctl` agent skill is to copy
`SKILL.md` into `~/.agents/skills/icalctl-skill/`:

```sh
mkdir -p ~/.agents/skills/icalctl-skill
cp SKILL.md ~/.agents/skills/icalctl-skill/SKILL.md
```

Run these commands from the project root. Copy the file again after updating
the repository to keep the installed skill current.

## Permissions

`icalctl` embeds separate Calendar and Reminders privacy usage strings in the
Mach-O binary through `embed_plist`. macOS authorizes the two EventKit stores
independently.

Run the first permission test from a normal terminal such as Terminal.app,
iTerm, or Ghostty:

```sh
icalctl calendars
icalctl reminders lists
```

If permission was granted, this should report your calendars. If the command was
launched from an embedded tool environment, macOS may attribute the permission
request to that parent process instead of the terminal.

Check current authorization status:

```sh
icalctl version
icalctl status
icalctl status --json
icalctl reminders status --json
icalctl doctor --json
```

`doctor` reports both authorization states, executable path and process id,
launch terminal, embedded Info.plist keys, and status-specific next commands.
If an embedded launcher cannot show a privacy prompt, run `icalctl calendars`
or `icalctl reminders lists` from Terminal.app, iTerm, or Ghostty.

For a stale denied permission entry, first try enabling Full Calendar Access in
System Settings > Privacy & Security > Calendars. If necessary, reset this
binary's Calendar decision and request it again from a normal terminal:

```sh
tccutil reset Calendar dev.zyd.icalctl
icalctl calendars
icalctl doctor --json
```

The equivalent Reminders reset is:

```sh
tccutil reset Reminders dev.zyd.icalctl
icalctl reminders lists
icalctl doctor --json
```

## Commands

```sh
icalctl status
icalctl doctor
icalctl calendars
icalctl default-calendar
icalctl today
icalctl upcoming --days 7
icalctl list --from 2026-07-07 --to 2026-07-07
icalctl search meeting --from 2026-07-07 --to 2026-07-14
icalctl show <event-id-or-row>
icalctl show <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00
icalctl add "Meeting" --start 2026-07-07T09:00 --end 2026-07-07T09:30
icalctl batch add --file events.json --if-exists skip --dry-run
icalctl update <event-id-or-row> --location "Library"
icalctl update <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00 --scope future --location "Library"
icalctl delete <event-id-or-row>
icalctl delete <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00 --scope occurrence
icalctl reminders status
icalctl reminders lists
icalctl reminders default-list
icalctl reminders list
icalctl reminders search report
icalctl reminders show <reminder-id-or-row>
icalctl reminders add "Submit report" --list-id LIST_ID --due 2026-07-15 --dry-run
```

Use command-specific help for the full option set:

```sh
icalctl add --help
icalctl batch add --help
icalctl update --help
icalctl delete --help
icalctl reminders --help
```

`icalctl show` loads event recurrence rules and their end conditions. It also
reports `is_detached` and the original `occurrence_date` for recurrence
exceptions. Recurring-event creation is supported through `add`; update and
delete require an explicit `--scope occurrence|future` for recurring targets.
A fresh cached row preserves the selected occurrence start. An EventKit series
identifier alone may resolve to its first occurrence, so pair a durable series
id with `--occurrence-start <RFC3339>` when inspecting or mutating a specific
occurrence. A cached row supplies that discriminator, but never supplies scope.

Create a recurring event through the normal add pipeline:

```sh
icalctl add "Biweekly review" --calendar-id CALENDAR_ID \
  --start 2026-07-20T09:00:00+03:00 \
  --end 2026-07-20T09:30:00+03:00 \
  --repeat weekly --repeat-interval 2 \
  --repeat-weekday monday --repeat-count 8 \
  --dry-run --json
```

`--repeat` accepts `daily`, `weekly`, `monthly`, or `yearly`. Repeatable
`--repeat-weekday` adds weekday constraints; `--repeat-month-day` is valid only
for monthly rules and accepts positive or negative month days;
finish with either `--repeat-count` or offset-bearing `--repeat-until`. The
normal calendar selection, validation, alarms, timezone, dry-run, and
confirmation safeguards still apply.
For a timed series that must retain a local wall-clock time across daylight
saving changes, pass its IANA `--time-zone`; EventKit stores that zone on the
series while explicit input offsets remain authoritative for parsing. Recurring
all-day events require date-only `--start` and `--end` values so every
occurrence remains a calendar date across offset transitions.
`--if-exists skip` is retry-safe only when the existing event has the exact
same normalized recurrence rule. A same-time event with a different or absent
rule is an explicit collision. Recurring `--if-exists update` remains blocked
because the add/reconcile command has no occurrence target; use `update` with
an exact occurrence and explicit scope instead.

Structured event JSON accepts the same nested rule:

```json
"recurrence": {
  "frequency": "weekly",
  "interval": 2,
  "weekdays": ["monday", "wednesday"],
  "count": 8
}
```

Batch `defaults` and individual events accept `recurrence` in that shape. A row
may use `"recurrence": null` to opt out of an inherited default. Batch dry-run,
preflight blocking, exact-calendar targeting, and partial-write safeguards are
unchanged.

## Date Input

Date-only values use the Mac's local timezone unless `add` or `update` supplies
`--time-zone`, in which case their midnight boundaries use that IANA zone.

For `--from`, a date-only value starts at local midnight:

```sh
--from 2026-07-07
```

For `--to`, a date-only value includes the whole day by converting internally to
the next local midnight:

```sh
icalctl list --from 2026-07-07 --to 2026-07-07
```

Accepted datetime forms include:

```text
2026-07-07
2026-07-07T09:30
2026-07-07T09:30:00
2026-07-07 09:30
2026-07-07T09:30:00+08:00
```

Offset inputs represent absolute instants. Write and dry-run JSON preserves the
original strings as `start_input` and `end_input`, and also reports
`start_utc`, `end_utc`, `start_local`, `end_local`, and `duration_seconds`.

For example, these equal displayed clock times are one hour apart:

```sh
icalctl add "Flight" \
  --start 2026-07-12T15:55:00+03:00 \
  --end 2026-07-12T15:55:00+02:00 \
  --time-zone Europe/Berlin \
  --dry-run --json
```

Datetime precedence is:

- No `--time-zone` + timezone-less input: use the Mac's local timezone.
- No `--time-zone` + explicit offset: use the explicit offset.
- `--time-zone` + timezone-less input: interpret it in the named IANA zone.
- `--time-zone` + explicit offset: use the explicit offset and store the named zone for EventKit display.

Timezone-less times that fall in a DST gap or overlap are rejected; provide an
explicit offset to select an unambiguous instant.

For example,
`--start 2026-07-12T15:55 --time-zone Europe/Berlin` means 15:55 in Berlin,
not on the Mac's local clock. On update, timezone-less inputs use the supplied
`--time-zone`; without that flag they use Mac local time even if the event
already has stored timezone metadata. The existing stored timezone remains
unchanged unless `--time-zone` or `--clear-time-zone` is passed. Use
`update <id> --clear-time-zone` to remove it.

EventKit does not store separate departure and arrival timezone names, so
travel workflows should use offset-bearing start/end inputs; their distinct
offsets remain in the input echo fields.

## JSON Output

Pass `--json` to any command:

```sh
icalctl today --json
icalctl calendars --json
icalctl default-calendar --json
icalctl add "Meeting" --start 2026-07-07T09:00 --end 2026-07-07T09:30 --json
```

JSON is compact by default and is the intended scripting interface.

`calendars --json` marks the EventKit default target with
`is_default_for_new_events: true`. `default-calendar --json` returns that
calendar directly, including its id, source, type, and writability.

Event JSON includes the selected calendar's title, EventKit id, source title,
and source id as `calendar`, `calendar_id`, `calendar_source`, and
`calendar_source_id`. It also reports `calendar_type`,
`allows_calendar_modifications`, normalized UTC/local instants,
`duration_seconds`, availability, and `has_notes`/`has_url`. Detail and live
write responses load alarms and include `alarm_count`; list responses use null
when alarms were not loaded. A successful `add --json` also reports
`calendar_selection` as `explicit` or `eventkit_default`.

Event read-back JSON always includes UTC/local timestamps and duration. When
EventKit reports an item timezone, it also includes
`start_in_event_time_zone` and `end_in_event_time_zone`.
Event list/search responses leave `recurrence_count` and `recurrence_rules`
null; `show` and live write readbacks load them. Event recurrence rules report
frequency, interval, termination, first weekday, ordinal weekday objects as
`{weekday, week_number}`, month days, months, year weeks/days, and set
positions. `is_detached` and `occurrence_date` identify an edited recurrence
exception and its original series instant.

Reminder commands use dedicated top-level types: `reminder_status`,
`reminder_lists`, `default_reminder_list`, `reminders`, and `reminder`.
`due` and `start` are nested values with `kind: "date"` or
`kind: "datetime"`. Date-only values keep their calendar date without a fake
midnight instant. Timed values include the original local components,
EventKit timezone when present, normalized offset-bearing value, and UTC value.
Reminder list/search responses deliberately leave alarm and recurrence counts
and rule collections null; `reminders show` loads the public alarm and
recurrence details.
Reminder creation previews use `reminder_dry_run` with `would_write: false`,
the exact resolved list and source ids, list-selection provenance, date inputs,
priority, field presence, planned notifications, duplicate policy, and planned
operation. `planned_alarms` is the complete preview collection for due-relative,
arbitrary absolute, and geofence alarms; legacy `notifications` contains only
due-relative time notifications. The preview also reports the single recurrence
rule when supplied. Lifecycle previews use `reminder_mutation_dry_run` with before/result
reminder objects and `changed_fields`; successful deletion uses
`reminder_deleted` with the deleted reminder and list ids.
Reminder batch output uses `reminder_batch`, schema version 1, per-row
`index`, `client_id`, `status`, `reminder_id`, `matched_reminder_id`, `draft`,
and `error` fields, and the same summary counters as event batches. Consumers
should treat a nonzero failed or not-attempted count as an unsuccessful command
result.

The current JSON contract is schema generation 1. Consumers should dispatch on
the top-level `type`, treat documented fields as stable, and tolerate additive
fields. A removal, rename, or semantic change requires an explicitly documented
schema-generation change; null means the value was unavailable or deliberately
not loaded, not a fabricated default.

## Calendar Selection

Calendar ids are the safest selectors for scripts and agents:

```sh
icalctl today --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D --json
icalctl add "Project sync" \
  --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30
```

`list`, `today`, `upcoming`, and `search` accept `--calendar-id` more than
once. Title selectors remain available for interactive use. A title that
matches more than one calendar now fails and lists each candidate's source,
source id, calendar id, and writability. Qualify a duplicate title by source:

```sh
icalctl today --calendar-source iCloud --calendar Calendar
icalctl today --source-id SOURCE_ID --calendar Calendar
```

Filter calendar discovery itself by exact source title and writability:

```sh
icalctl calendars --source iCloud --writable-only --json
```

Before relying on the implicit EventKit default, inspect it explicitly:

```sh
icalctl default-calendar --json
```

## Dry-Run Write Planning

Add `--dry-run` to `add` or `update` to resolve and validate the complete event
without changing Calendar:

```sh
icalctl add "Project sync" \
  --calendar-id CALENDAR_ID \
  --start 2026-07-10T09:00 \
  --end 2026-07-10T09:30 \
  --alarm-minutes-before 10 \
  --dry-run \
  --json

icalctl update EVENT_ID --location "Room 3" --dry-run --json
```

The JSON response has `type: "dry_run"` and `would_write: false`. Its `draft`
includes the resolved calendar and source ids, normalized start/end values,
all-day/timed state, availability, resulting alarm count, notes/location/URL
presence, and exact duplicate warnings. Dry-run performs the same calendar,
date-range, URL, availability, and alarm validation as a live write.

## Batch Event Imports

Use `batch add` to validate and import multiple events from one versioned JSON
file. Preview the complete batch before running the live command:

```sh
icalctl batch add --file events.json --if-exists skip --dry-run --json
icalctl batch add --file events.json --if-exists skip --json
```

The file uses a top-level object with optional shared defaults:

```json
{
  "version": 1,
  "defaults": {
    "calendar_id": "CALENDAR_ID",
    "time_zone": "Europe/Berlin",
    "availability": "busy",
    "alarm_minutes_before": [10]
  },
  "events": [
    {
      "client_id": "flight-outbound",
      "title": "Flight to Berlin",
      "start": "2026-07-12T15:55",
      "end": "2026-07-12T18:10",
      "location": "PVG"
    }
  ]
}
```

Each event requires `title`, `start`, and `end`. Events may override the shared
calendar selector, timezone, availability, all-day state, and alarms. Calendar
selectors use the same `calendar`, `calendar_id`, `calendar_source`, and
`source_id` fields as the CLI. `client_id` is returned for correlation but is
not stored in EventKit.
Recurring rows whose effective `all_day` value is true require date-only
`start` and `end`, including when recurrence or all-day state comes from
`defaults`.

Existing events are matched exactly by resolved calendar id, title, normalized
start and end instants, and all-day state. `--if-exists` defaults to `error`;
choose `skip` for idempotent reruns or `update` to patch optional fields on the
matching event. With `update`, omitted fields remain unchanged, explicit `null`
clears `notes`, `location`, `url`, or `time_zone`, and a supplied
`alarm_minutes_before` array replaces all existing alarms.
For recurring matches, `skip` requires the identical normalized rule and
batch `update` is rejected because the batch row has no exact occurrence and
scope; use the standalone `update` command.

By default, any preflight error blocks every write. `--continue-on-error`
processes valid items and continues after individual write failures. EventKit
does not provide a multi-event transaction, so a runtime failure cannot roll
back events that were already written. Batch JSON reports every item as
created, skipped, updated, failed, not attempted, or the corresponding
`would_*` dry-run status, and exits nonzero if any item failed.

## Flight Event Helper

`travel flight` formats one flight leg and routes it through the same add,
calendar-selection, duplicate, dry-run, and EventKit write pipeline as a
generic event:

```sh
icalctl travel flight HO1607 \
  --from PVG \
  --to HEL \
  --departure 2026-07-11T09:25:00+08:00 \
  --arrival 2026-07-11T14:00:00+03:00 \
  --calendar-id CALENDAR_ID \
  --if-exists skip \
  --dry-run --json
```

Flight numbers and 3-4 letter airport codes are trimmed and uppercased. Both
timestamps must be RFC3339 values with explicit offsets, and arrival must be
after departure as an absolute instant. The helper never infers airport
timezones and does not store one EventKit item timezone, because a flight has
two local zones. Offset-bearing input echoes and generated notes remain the
authoritative local-time record.

The helper deterministically generates:

```text
Title: Flight HO1607: PVG to HEL
Location: PVG to HEL
Availability: busy
Alarms: none

Flight: HO1607
Route: PVG to HEL
Departure: PVG 2026-07-11T09:25:00+08:00
Arrival: HEL 2026-07-11T14:00:00+03:00
```

Use `--notes` or `--notes-file` for extra text appended after one blank line.
The helper also accepts existing calendar selectors, URL, availability, alarm,
duplicate-policy, duplicate-window, and dry-run flags. It does not fetch
airline data, status, gates, terminals, bookings, delays, or airport metadata.

The equivalent generic invocation is:

```sh
icalctl add "Flight HO1607: PVG to HEL" \
  --start 2026-07-11T09:25:00+08:00 \
  --end 2026-07-11T14:00:00+03:00 \
  --location "PVG to HEL" \
  --notes $'Flight: HO1607\nRoute: PVG to HEL\nDeparture: PVG 2026-07-11T09:25:00+08:00\nArrival: HEL 2026-07-11T14:00:00+03:00' \
  --availability busy \
  --calendar-id CALENDAR_ID \
  --if-exists skip \
  --dry-run --json
```

For multiple legs, keep using ordinary batch JSON rather than a separate
travel schema. This is the canonical recipe:

```json
{
  "version": 1,
  "defaults": {
    "calendar_id": "CALENDAR_ID",
    "availability": "busy",
    "alarm_minutes_before": []
  },
  "events": [
    {
      "client_id": "ho1607-pvg-hel",
      "title": "Flight HO1607: PVG to HEL",
      "start": "2026-07-11T09:25:00+08:00",
      "end": "2026-07-11T14:00:00+03:00",
      "location": "PVG to HEL",
      "notes": "Flight: HO1607\nRoute: PVG to HEL\nDeparture: PVG 2026-07-11T09:25:00+08:00\nArrival: HEL 2026-07-11T14:00:00+03:00"
    },
    {
      "client_id": "ay1415-hel-fra",
      "title": "Flight AY1415: HEL to FRA",
      "start": "2026-07-12T07:40:00+03:00",
      "end": "2026-07-12T09:20:00+02:00",
      "location": "HEL to FRA",
      "notes": "Flight: AY1415\nRoute: HEL to FRA\nDeparture: HEL 2026-07-12T07:40:00+03:00\nArrival: FRA 2026-07-12T09:20:00+02:00"
    }
  ]
}
```

## Apple Reminders

Reminders access is separate from Calendar access. Check it and inspect the
available reminder lists with:

```sh
icalctl reminders status --json
icalctl reminders lists --json
icalctl reminders lists --source iCloud --writable-only --json
icalctl reminders default-list --json
```

List and search default to incomplete reminders. Use `--state completed` or
`--state all` when needed:

```sh
icalctl reminders list --json
icalctl reminders list --state all --list-id LIST_ID --json
icalctl reminders search "report" --state completed --json
```

`--list` and `--list-id` are repeatable. Exact ids are preferred for scripts.
A duplicate title fails with candidate list ids, sources, source ids, and
writability; qualify it with `--list-source` or `--source-id`:

```sh
icalctl reminders list --list Tasks --list-source iCloud --json
icalctl reminders list --list Tasks --source-id SOURCE_ID --json
```

Due filters accept the same date and datetime forms as event range inputs.
They exclude undated reminders; a date-only `--due-to` includes that entire
local date:

```sh
icalctl reminders list --due-from 2026-07-10 --due-to 2026-07-15 --json
```

Inspect one reminder by exact EventKit id or by a row from the latest reminder
list/search:

```sh
icalctl reminders show REMINDER_ID --json
icalctl reminders show 1 --json
```

Create an undated, date-only, or timed reminder only after inspecting the
available writable lists and previewing the exact target:

```sh
icalctl reminders lists --writable-only --json
icalctl reminders add "Submit report" \
  --list-id LIST_ID \
  --due 2026-07-15 \
  --priority high \
  --dry-run --json
```

Use a strict structured JSON draft when shell flags are inconvenient:

```sh
icalctl reminders add --json-file reminder.json --if-exists skip --dry-run --json
```

Example `reminder.json`:

```json
{
  "title": "Review budget",
  "list_id": "LIST_ID",
  "due": "2026-07-15T09:00:00+03:00",
  "notes": "Review the final numbers",
  "priority": "high",
  "notify_at": ["2026-07-15T07:00:00+03:00"],
  "recurrence": {
    "frequency": "monthly",
    "interval": 1,
    "count": 12
  }
}
```

The object rejects unknown fields and supports the same list selectors,
due/start/timezone values, content, priority, alarms, geofence, and recurrence
as flag-based add. `client_id` is reserved for batch rows. Keep duplicate,
dry-run, and output policy on the command. Do not combine `--json-file` with a
positional title or individual reminder fields.

Omitting `--due` creates an undated reminder. `YYYY-MM-DD` remains a true
date-only value. For timezone-less timed values, `--time-zone` selects the IANA
zone; without it, the Mac local zone is used. An explicit RFC3339 offset always
wins, including when `--time-zone` is also present. The dry run echoes the
input, local components, normalized offset-bearing value, UTC value, and
EventKit component timezone.

The add command also accepts `--start`, `--notes` or `--notes-file`, `--url`,
`--location`, and `--priority none|low|medium|high`. Priority is independent
from notifications and never creates one by itself.
Free-text location uses the public EventKit field; provider-backed lists may
normalize or drop it, so the live JSON response is the authoritative readback.

For a timed due value, add deterministic notifications at or before the due
instant:

```sh
icalctl reminders add "Call the dentist" \
  --list-id LIST_ID \
  --due 2026-07-15T14:30 \
  --time-zone Europe/Helsinki \
  --notify-at-due \
  --notify-minutes-before 30 \
  --dry-run --json
```

`--notify-minutes-before` is repeatable and requires a positive number.
Notification flags require a timed due value; date-only and undated reminders
do not have a deterministic notification instant. No notifications are added
by default. The CLI computes absolute EventKit alarm times and reports both UTC
and due-timezone values in dry-run JSON.

Duplicate identity is exact list id, title, and due kind/value. The default
policy is `--if-exists error`; `skip` returns the existing reminder and
`update` patches only supplied non-identity fields. Timed due matching is exact
unless `--duplicate-window-seconds` is explicitly nonzero; date-only and
undated identities always remain exact. When notification flags are supplied
with `--if-exists update`, they replace existing alarms; omission preserves
existing alarms.

Add arbitrary absolute notification instants independently of the due value
with repeatable offset-bearing `--notify-at` values:

```sh
icalctl reminders add "Prepare documents" \
  --list-id LIST_ID \
  --due 2026-07-15 \
  --notify-at 2026-07-14T09:00:00+03:00 \
  --dry-run --json
```

Add one arrival or departure geofence alarm by supplying the complete location
tuple. Coordinates and radius are never inferred from the title:

```sh
icalctl reminders add "Collect package" \
  --list-id LIST_ID \
  --geofence-title "Post office" \
  --geofence-latitude 60.1699 \
  --geofence-longitude 24.9384 \
  --geofence-radius-meters 150 \
  --geofence-proximity arrive \
  --dry-run --json
```

Latitude must be from -90 through 90, longitude from -180 through 180, and
radius must be positive. `icalctl doctor --json` reports Location Services,
authorization, and the embedded usage description for diagnostics. These
statuses do not block an explicit-coordinate geofence write: icalctl constructs
the EventKit location without reading the device's current location. Actual
trigger delivery remains controlled by macOS and Reminders settings.

An ignored, explicit-opt-in EventKit integration test can create, read back,
clear, and delete an advanced reminder only in an exact writable
`icalctl Test` list. Set `ICALCTL_RUN_REMINDER_EVENTKIT_TESTS=1` and
`ICALCTL_TEST_REMINDER_LIST_ID` before running that single ignored test; normal
test runs never write Reminders data.

Create multiple reminders with a versioned batch file:

```sh
icalctl reminders batch add --file reminders.json \
  --if-exists skip --dry-run --json
```

Canonical `reminders.json` shape:

```json
{
  "version": 1,
  "defaults": {
    "list_id": "LIST_ID",
    "time_zone": "Europe/Helsinki",
    "priority": "medium"
  },
  "reminders": [
    {
      "client_id": "submit-report",
      "title": "Submit report",
      "due": "2026-07-15"
    },
    {
      "client_id": "call-dentist",
      "title": "Call dentist",
      "due": "2026-07-16T14:30",
      "notify_minutes_before": [30]
    }
  ]
}
```

Batch defaults support list selection, timezone, priority, alarms, geofence,
and recurrence. Rows override defaults with their own values. Every row is
strict and must have a unique `client_id` when one is supplied. Duplicate
resolved list/title/due identities within the file fail preflight.
An inherited timezone applies only to timezone-less timed due/start values, so
date-only and undated rows can safely share the same batch. Set a row's
`time_zone`, `geofence`, or `recurrence` to `null` to opt out of that default;
omitting the field inherits it. Live writes are pinned to the exact list id
resolved and reported by preflight.

By default, any preflight error blocks all writes. `--continue-on-error` allows
valid rows to proceed despite invalid rows or later write failures; disclose
that partial-write behavior before confirmation because EventKit cannot roll
back earlier reminders. Confirm every planned create or update and exact target
list before the live batch invocation.

Create one simple recurrence rule anchored by a due or start date:

```sh
icalctl reminders add "Review budget" \
  --list-id LIST_ID \
  --due 2026-07-15T09:00:00+03:00 \
  --repeat monthly \
  --repeat-interval 1 \
  --repeat-count 12 \
  --dry-run --json
```

`--repeat` accepts `daily`, `weekly`, `monthly`, or `yearly`.
`--repeat-interval` defaults to 1. Use either positive `--repeat-count` or an
offset-bearing RFC3339 `--repeat-until`, not both. The simple rule follows the
due/start anchor; advanced BYDAY/BYMONTH patterns are not exposed.

If all list selectors are omitted, EventKit's default reminder list is used.
Inspect `reminders default-list --json` first and still confirm its exact title
and id. A dry run does not replace user confirmation. Only after confirmation,
repeat the same command without `--dry-run`.

Patch an existing reminder by exact id or by a row from the latest reminder
list/search. Omitted fields remain unchanged. Nullable fields use explicit
clear flags, and `--priority none` clears priority:

```sh
icalctl reminders update REMINDER_ID \
  --title "Submit final report" \
  --due 2026-07-16T09:30 \
  --time-zone Europe/Helsinki \
  --clear-notes \
  --priority high \
  --dry-run --json
```

Use `--clear-due`, `--clear-start`, `--clear-time-zone`, `--clear-notes`,
`--clear-url`, or `--clear-location` to remove those values. `--time-zone`
sets timezone metadata on timezone-less supplied values and also applies to
unchanged timed due/start fields. Explicit RFC3339 offsets remain authoritative.
`--clear-time-zone` preserves wall-clock fields as floating EventKit components.
Moving a reminder uses the same exact, writable list selection rules as creation:

```sh
icalctl reminders update REMINDER_ID --list-id DESTINATION_LIST_ID --dry-run --json
```

Complete and uncomplete support zero-write previews. `--completed-at` must be
RFC3339 with an explicit offset; omission uses the current instant:

```sh
icalctl reminders complete REMINDER_ID \
  --completed-at 2026-07-11T14:00:00+03:00 --dry-run --json
icalctl reminders uncomplete REMINDER_ID --dry-run --json
```

Delete prompts for the word `delete` unless `--force` is supplied:

```sh
icalctl reminders delete REMINDER_ID
```

Supplying any notification/geofence option to `reminders update` replaces the
entire existing alarm collection. Use `--clear-notifications` to remove all
alarms. Supplying `--repeat` replaces the recurrence rule; use
`--clear-recurrence` to remove it. Omission preserves existing alarms and
recurrence.

Before any live update, move, completion change, or deletion, inspect the
exact reminder and target list, preview where supported, and obtain explicit
confirmation. Update currently preserves reminder alarms and recurrence rules;
their explicit replacement or clear options are the only exception.

The adapter uses only Apple's public EventKit API. Features that Reminders.app
does not expose publicly through EventKit—flags, tags, sections, subtasks,
attachments, templates, and messaging triggers—are not read through private
selectors or by editing the Calendar database.

## Row Cache

List-like commands cache their most recent event rows:

```sh
icalctl today
icalctl list --from 2026-07-07 --to 2026-07-07
icalctl upcoming --days 3
icalctl search meeting --from 2026-07-07 --to 2026-07-14
```

The human output includes row numbers:

```text
Events (2)
1. 2026-07-07 09:00-09:30 Meeting (Work) [EVENT_ID]
2. 2026-07-07 14:00-15:00 Class (School) [EVENT_ID]
```

Then `show`, `update`, and `delete` can use either the row number or the exact
EventKit identifier:

```sh
icalctl show 1
icalctl update 2 --title "Updated title"
icalctl delete 1
```

The cache is only a convenience mapping from row number to EventKit id. Calendar
data remains in Apple Calendar. The cache file is written to:

```text
~/Library/Caches/icalctl/last-events.json
```

or, when `XDG_CACHE_HOME` is set:

```text
$XDG_CACHE_HOME/icalctl/last-events.json
```

Reminder list/search rows use a different file so event rows can never resolve
as reminders (or vice versa):

```text
~/Library/Caches/icalctl/last-reminders.json
```

or `$XDG_CACHE_HOME/icalctl/last-reminders.json`.

## Creating Events

Basic timed event:

```sh
icalctl add "Project sync" \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30
```

Specify calendar, location, notes, URL, availability, and alarms:

```sh
icalctl add "Project sync" \
  --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30 \
  --location "Room 3" \
  --notes "Bring agenda" \
  --url "https://example.com" \
  --availability busy \
  --alarm-minutes-before 10
```

Preview the same event before writing:

```sh
icalctl add "Project sync" \
  --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30 \
  --dry-run --json
```

Single-event adds check for an existing event with the same calendar id, title,
normalized start/end instants, and all-day state. The safe default is
`--if-exists error`; use `skip` for retry-safe creation or `update` to patch the
matching event's supplied optional fields. Supplied alarms replace existing
alarms during an update. Matching is exact by default; opt into a start/end
tolerance with `--duplicate-window-seconds N`. Dry-run reports the planned
operation and matching event id without writing:

```sh
icalctl add "Project sync" \
  --calendar-id CALENDAR_ID \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30 \
  --if-exists skip \
  --dry-run --json
```

Live add JSON reports `write_action` as `created`, `skipped`, or `updated`.

For exact multiline notes, read UTF-8 text from a file or stdin. Contents are
not trimmed or newline-normalized:

```sh
icalctl add "Project sync" --start 2026-07-07T09:00 --end 2026-07-07T09:30 \
  --notes-file notes.txt
printf 'First line\nSecond line\n' | \
  icalctl add "Project sync" --start 2026-07-07T09:00 --end 2026-07-07T09:30 \
  --notes-file -
```

Use `--json-file` for a complete structured single-event draft. Individual
event fields cannot be mixed with the file; command-level duplicate, dry-run,
and JSON-output flags remain available:

```json
{
  "title": "Project sync",
  "start": "2026-07-07T09:00",
  "end": "2026-07-07T09:30",
  "calendar_id": "CALENDAR_ID",
  "notes": "Agenda",
  "location": "Room 3",
  "url": "https://example.com/event",
  "availability": "busy",
  "time_zone": "Asia/Shanghai",
  "alarm_minutes_before": [10],
  "timed": true
}
```

```sh
icalctl add --json-file event.json --dry-run --json
```

The JSON selector may instead use `calendar` with either `calendar_source` or
`source_id`. Set `all_day` or `timed`, but not both to true. Unknown fields and
conflicting selectors fail validation before Calendar writes.

All-day event:

```sh
icalctl add "Conference" --start 2026-07-07 --end 2026-07-09 --all-day
```

## Updating Events

Update by cached row:

```sh
icalctl today
icalctl update 1 --location "Library"
```

Clear nullable fields:

```sh
icalctl update 1 --clear-location --clear-notes --clear-url
```

Move to another calendar by exact id:

```sh
icalctl update 1 --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D
```

Add another alarm:

```sh
icalctl update 1 --add-alarm-minutes-before 30
```

Set or clear the EventKit item timezone:

```sh
icalctl update 1 --time-zone America/New_York
icalctl update 1 --clear-time-zone
```

For a recurring event, select the exact occurrence and choose whether the
change affects only it or it and every later occurrence:

```sh
icalctl update SERIES_ID \
  --occurrence-start 2026-07-20T09:00:00+03:00 \
  --scope occurrence --location "Library" --dry-run --json
icalctl update SERIES_ID \
  --occurrence-start 2026-07-20T09:00:00+03:00 \
  --scope future --location "Library" --dry-run --json
```

A fresh cached row can replace the id plus `--occurrence-start`, but
`--scope` is still mandatory. `--scope` is rejected for non-recurring events.

## Deleting Events

By default, delete asks for confirmation:

```sh
icalctl delete 1
```

Type `delete` when prompted. Use `--force` for scripts:

```sh
icalctl delete 1 --force
```

Recurring deletion likewise requires an exact occurrence and explicit scope:

```sh
icalctl delete SERIES_ID \
  --occurrence-start 2026-07-20T09:00:00+03:00 \
  --scope occurrence
icalctl delete SERIES_ID \
  --occurrence-start 2026-07-20T09:00:00+03:00 \
  --scope future
```

## Shell Completions

Print completion scripts to stdout:

```sh
icalctl completions zsh
icalctl completions bash
icalctl completions fish
icalctl completions elvish
icalctl completions powershell
```

Example zsh setup:

```sh
mkdir -p ~/.zfunc
icalctl completions zsh > ~/.zfunc/_icalctl
```

Then make sure `~/.zfunc` is in `fpath` before `compinit` in your zsh config.

## Development

Common checks:

```sh
cargo fmt --check
cargo check
cargo clippy -- -D warnings
cargo test
cargo build
```

Install locally:

```sh
cargo install --path .
```

Useful smoke tests that do not mutate Calendar data:

```sh
icalctl --help
icalctl status
icalctl calendars
icalctl today
icalctl reminders status
icalctl reminders lists
icalctl reminders list
icalctl completions zsh
```

The manual EventKit suite is ignored by ordinary `cargo test`. Its read-only
permission/default-calendar check can be run explicitly:

```sh
cargo test --test eventkit_manual permission_and_default_calendar_are_parseable \
  -- --ignored --nocapture
```

The round-trip tests perform real creates, read-backs, updates, and deletes.
Before using them, create a writable calendar named exactly `icalctl Test`,
inspect its exact id, and explicitly opt in:

```sh
ICALCTL_RUN_EVENTKIT_TESTS=1 \
ICALCTL_TEST_CALENDAR_ID=EXACT_TEST_CALENDAR_ID \
cargo test --test eventkit_manual \
  create_read_back_and_delete_on_explicit_test_calendar \
  -- --ignored --nocapture
```

The recurrence hardening scenario additionally verifies a Berlin DST boundary,
all-day date preservation, a detached occurrence, a future series split, and
both delete scopes:

```sh
ICALCTL_RUN_EVENTKIT_TESTS=1 \
ICALCTL_TEST_CALENDAR_ID=EXACT_TEST_CALENDAR_ID \
cargo test --test eventkit_manual \
  recurring_dst_all_day_and_scoped_mutations_on_explicit_test_calendar \
  -- --ignored --nocapture
```

The tests refuse any other calendar title and install cleanup guards, but they
still mutate real Calendar data and should only run against the dedicated test
calendar.

## Architecture

```text
clap CLI
  -> command dispatcher
  -> event adapter via eventkit-rs
  -> public reminder adapter via objc2-event-kit
  -> macOS Calendar and Reminders stores
```

Main modules:

- `src/cli.rs`: command and flag definitions
- `src/calendar.rs`: EventKit read/write operations
- `src/calendar_selector.rs`: stable id/source/title selector resolution
- `src/eventkit_bridge.rs`: exact-calendar-id EventKit writes through `objc2`
- `src/cache.rs`: last-list row cache
- `src/dates.rs`: local date parsing
- `src/models.rs`: JSON/report structs
- `src/output.rs`: human-readable formatting
- `src/reminders.rs`: testable read service, list selectors, filters, and
  public-only EventKit reminder bridge
- `src/travel.rs`: pure deterministic flight-to-event formatting
- `tests/eventkit_manual.rs`: opt-in real EventKit verification with strict safeguards

`eventkit-rs` remains the high-level event wrapper. Reminders use generated
`objc2-event-kit` bindings directly because this project must preserve
date-component semantics and avoid wrapper paths that inspect private reminder
properties.
