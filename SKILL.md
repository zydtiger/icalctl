---
name: icalctl
description: Use when managing local macOS Apple Calendar and Reminders data with the icalctl CLI, including reading/searching reminders, listing calendars and reminder lists, producing JSON, creating/updating/deleting events, and safely choosing and confirming targets before writes.
---

# icalctl

Use `icalctl` for local macOS Apple Calendar and Reminders automation through EventKit. It works with the accounts already configured in Calendar.app and Reminders.app, including iCloud, Google, Exchange, and local accounts.

This project targets Apple Silicon macOS (`aarch64-apple-darwin`). It is for the local Apple Calendar store, not remote Google Calendar APIs.

Install from the project root with:

```sh
cargo install --path .
```

Verify which build is active before relying on newly added flags:

```sh
icalctl version
icalctl version --json
```

Treat `Cargo.toml`'s package version as the semantic-version source. The
version report also includes the embedded Git commit when available, target
triple, and build profile. Reinstall an updated checkout with
`cargo install --path . --force`; copying `SKILL.md` does not update the binary.

## Operating Rules

- Prefer `icalctl <command> --json` whenever output must be parsed by an agent or script.
- Use human-friendly output when answering a person directly and JSON when making decisions from command output.
- Treat event `add`, live `batch add`, `update`, `delete`, and reminder add/update/complete/uncomplete/delete commands without `--dry-run` as live user-data writes.
- Do not create, update, move, or delete an event until the user has explicitly confirmed the final action.
- For every request to add an event, analyze the best-fit calendar from the user's available calendars and confirm that calendar choice before writing.
- Prefer exact `--calendar-id` selectors for agent writes and reads. Title-only selectors are acceptable only when the title is unique.
- Do not create, update, move, complete, uncomplete, or delete a reminder until the user has explicitly confirmed the exact reminder, target reminder-list title/id when applicable, and final change. This is required for dated reminders just as calendar selection is required for events.
- Prefer exact `--list-id` selectors for reminder writes and reads. A title-only reminder-list selector is acceptable only when the title is unique; otherwise qualify it by source.
- Use `icalctl reminders default-list --json` before any workflow that intentionally relies on EventKit's implicit default reminder list.
- Use `icalctl default-calendar --json` before any workflow that intentionally relies on EventKit's implicit default target.
- Prefer `--dry-run --json` to validate and preview event or reminder writes before asking for final confirmation.
- For timezone-less event inputs, `--time-zone <TZID>` controls parsing and stores the same single EventKit timezone. For offset-bearing or travel times, explicit offsets remain authoritative; verify input echoes, UTC fields, and `duration_seconds`.
- Quote titles, calendar names, notes, locations, and URLs that contain spaces or shell metacharacters.
- Use row numbers only immediately after a fresh `today`, `upcoming`, `list`, or `search`; otherwise use the exact EventKit id or rerun the list command. A recurring series id does not distinguish its occurrences: use a fresh row or pair the id with `--occurrence-start <RFC3339>` for a specific occurrence. Before recurring update/delete, separately confirm `--scope occurrence|future`; a row never chooses scope.
- Use reminder row numbers only immediately after a fresh `reminders list` or `reminders search`. Event and reminder rows use different cache files and cannot be interchanged.
- Do not substitute a fake calendar event when the user asks for a reminder.

If `icalctl` is not installed in `PATH`, run it from the project with:

```sh
cargo run -- <command>
```

## Permission Check

Start with authorization when Calendar access is uncertain:

```sh
icalctl status
icalctl status --json
icalctl doctor --json
```

Reminders authorization is separate:

```sh
icalctl reminders status
icalctl reminders status --json
icalctl reminders lists
```

Useful statuses:

- `FullAccess`: read and write commands can work.
- `WriteOnly`: current `icalctl` requires full access for all Calendar-touching commands, including adds, because it reads and validates calendars before writes.
- `Denied` or `Restricted`: the user must change macOS privacy settings.
- `NotDetermined`: run a real Calendar command from Terminal.app, iTerm, or Ghostty so macOS can show the permission prompt.

To trigger the permission prompt and verify calendars:

```sh
icalctl calendars
```

Use `doctor --json` when access fails. It reports authorization, process and
launch context, both Calendar and Reminders Info.plist keys, and recommended
next commands. For
`NSMachErrorDomain` or Mach error 4099, rerun `icalctl calendars` from
Terminal.app, iTerm, or Ghostty. For a stale denied entry, enable access in
System Settings first; if needed, run
`tccutil reset Calendar dev.zyd.icalctl` and request access again.

For a stale Reminders decision, enable access in System Settings first; if
needed, run `tccutil reset Reminders dev.zyd.icalctl`, then request access with
`icalctl reminders lists` from a normal terminal.

## Required Workflow for Adding Events

When the user asks to add an event, do this sequence:

1. Inspect available calendars:

```sh
icalctl calendars --json
```

2. Infer the best target calendar from the event category and the actual calendar names available.

Use these heuristics:

- Work meetings, client calls, interviews, office tasks, project syncs: choose a work, company, professional, or project calendar if present.
- Classes, homework, research, academic deadlines, campus events: choose a school, university, study, academic, or research calendar if present.
- Personal errands, family, friends, meals, social plans, hobbies: choose a personal, home, life, or default personal calendar if present.
- Medical, workouts, health appointments: choose a health, exercise, fitness, or personal calendar if present.
- Flights, hotels, trips, visas, itinerary items: choose a travel calendar if present, otherwise personal.
- Bills, subscriptions, banking, tax, investing, trading, financial reminders: choose a finance calendar if present.
- Birthdays, anniversaries, holidays: choose a birthdays, holidays, personal, or family calendar if present.
- If the title or context names a specific calendar, prefer that calendar, but still confirm before writing.
- If multiple calendars are plausible, present the top candidates and ask the user to choose.

3. Preview the complete event without writing:

```sh
icalctl add "Project sync" \
  --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30 \
  --alarm-minutes-before 10 \
  --dry-run \
  --json
```

Inspect the resolved calendar, normalized times, availability, alarm count, field-presence flags, and duplicate warnings. A dry run has `would_write: false` and does not replace final user confirmation.

4. Confirm the complete write with the user before running live `icalctl add`.

The confirmation must include:

- calendar title
- calendar id
- event title
- start and end date/time, including timezone assumptions
- stored EventKit time zone, if any
- whether it is all-day or timed
- notes, location, URL, availability, and alarms if any

Example confirmation:

```text
I would add "Project sync" to the Work calendar on 2026-07-07 from 09:00 to 09:30 local time, with a 10 minute alarm. Confirm?
```

5. Only after confirmation, run the add command without `--dry-run`:

```sh
icalctl add "Project sync" \
  --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D \
  --start 2026-07-07T09:00 \
  --end 2026-07-07T09:30 \
  --alarm-minutes-before 10 \
  --json
```

6. Report the created event id, title, time, calendar id, and calendar source.

## Required Workflow for Adding Reminders

When the user asks to add a reminder:

1. Inspect actual writable reminder lists:

```sh
icalctl reminders lists --writable-only --json
```

2. Choose the best existing list from its title, id, source, and writability.
Prefer an exact `--list-id`. If the workflow intentionally uses EventKit's
default, inspect it first with `icalctl reminders default-list --json`.
Duplicate list titles must be qualified or selected by exact id.

3. Preview the complete reminder without writing:

```sh
icalctl reminders add "Submit report" \
  --list-id LIST_ID \
  --due 2026-07-15 \
  --priority high \
  --dry-run --json
```

4. Inspect the resolved list and source ids, list-selection provenance, title,
due/start kind and normalized times, priority, notes/location/URL presence,
planned notification count and absolute times, duplicate match, and planned
operation.

5. Ask the user to confirm the exact reminder-list title and id before the live
write. For every dated reminder, also confirm whether the due value is
date-only or timed, its displayed time and timezone/offset assumption, start
value if any, priority, notes, location, URL, and notifications. Priority never
implies a notification.

6. Only after confirmation, repeat the command without `--dry-run`, preferably
with the exact `--list-id`, then report the reminder id, title, due value, list
title/id/source, priority, and write action.

A dry run never replaces user confirmation. Use `--if-exists skip` for
retry-safe creation only after previewing the match. Use `update` only when the
user has confirmed the supplied non-identity patches.

For a strict structured reminder draft, use
`icalctl reminders add --json-file FILE --dry-run --json`. Do not combine the
file with a title or individual reminder fields. Inspect every resolved field
and exact list just as for flag-based creation.

For a versioned reminder batch, use
`icalctl reminders batch add --file FILE --dry-run --json`. Inspect every row,
resolved list id, planned action, alarm, recurrence, and preflight error. Require
unique `client_id` values when supplied. Any preflight error blocks all writes
unless `--continue-on-error` is explicit; disclose that it permits partial
writes and obtain confirmation for every planned create or update before the
live command. Prefer `--if-exists skip` for retry-safe batches.

## Required Workflow for Reminder Lifecycle Changes

For reminder updates, moves, completion changes, and deletion:

1. Resolve a fresh exact target with `icalctl reminders show ID --json`. Use a
   reminder row only immediately after a fresh reminder list/search.
2. For a list move, inspect `icalctl reminders lists --writable-only --json`
   and select the destination by exact `--list-id` whenever possible.
3. Preview updates, completion, and uncompletion with `--dry-run --json`.
   Confirm the before/result reminder, changed fields, dates/timezones,
   priority, and exact destination list. Omitted update fields remain unchanged;
   nullable fields require their explicit `--clear-*` option, while
   `--priority none` clears priority.
4. Ask the user for explicit confirmation before the live command. A dry run is
   not consent.
5. For deletion, show the exact reminder and ask for confirmation before
   invoking `reminders delete`; use `--force` only after that confirmation.

Examples:

```sh
icalctl reminders update ID --due 2026-07-16T09:30 \
  --time-zone Europe/Helsinki --clear-notes --dry-run --json
icalctl reminders update ID --list-id DESTINATION_LIST_ID --dry-run --json
icalctl reminders complete ID \
  --completed-at 2026-07-11T14:00:00+03:00 --dry-run --json
icalctl reminders uncomplete ID --dry-run --json
icalctl reminders delete ID
```

`--completed-at` requires RFC3339 with an explicit offset; omission uses the
current instant. `--clear-time-zone` preserves timed due/start wall-clock fields
while removing their EventKit timezone metadata. Omitted alarm and recurrence
fields remain unchanged. Any supplied notification or geofence option replaces
the complete alarm collection; `--clear-notifications` removes it. `--repeat`
replaces the single recurrence rule and `--clear-recurrence` removes it.

For arbitrary time alarms, use repeatable offset-bearing
`--notify-at <RFC3339>`. For a location alarm, require the complete explicit
tuple: `--geofence-title`, latitude, longitude, positive radius in meters, and
`--geofence-proximity arrive|leave`. Never infer coordinates. Run
`icalctl doctor --json` for Location diagnostics. Treat those fields as
informational for explicit-coordinate geofences because icalctl does not read
the device's current location; macOS still controls eventual trigger delivery.
For recurrence, require a due/start anchor and use
`--repeat daily|weekly|monthly|yearly`, an optional positive interval, and either a positive
count or offset-bearing end instant. Include every alarm, geofence coordinate,
radius, proximity, and recurrence end in the user confirmation.

## Date And Time Input

Accepted forms include:

```text
2026-07-07
2026-07-07T09:30
2026-07-07T09:30:00
2026-07-07 09:30
2026-07-07T09:30:00+08:00
```

Date-only values use the Mac's local timezone unless `add` or `update` supplies `--time-zone`, in which case their midnight boundaries use that IANA zone. For range commands, date-only `--from` starts at local midnight, and date-only `--to` includes the whole day by internally ending at the next local midnight.

Offset-bearing inputs are absolute instants. Write and dry-run JSON preserves them as `start_input` and `end_input` and reports normalized UTC/local values plus `duration_seconds`. Human event times include their UTC offset.

Datetime precedence is: no `--time-zone` plus timezone-less input uses Mac local time; no `--time-zone` plus an explicit offset uses that offset; `--time-zone` plus timezone-less input uses the named IANA zone; and `--time-zone` plus an explicit offset still uses that offset while storing the named zone for EventKit display. On update, omitting `--time-zone` means naive inputs use Mac local time even if the event already has timezone metadata; that stored metadata remains unchanged. Timezone-less values in DST gaps or overlaps are rejected, so use an explicit offset for an unambiguous instant. EventKit does not preserve separate start/end timezone names, so travel events should use offset-bearing start/end inputs and verify the echoed inputs and duration.

Example: `2026-07-12T15:55:00+03:00` to `2026-07-12T15:55:00+02:00` is a one-hour event even though both clocks show `15:55`.

For all-day events, pass date-only values with `--all-day`:

```sh
icalctl add "Conference" --calendar-id CALENDAR_ID --start 2026-07-07 --end 2026-07-09 --all-day
```

All-day start and end dates are user-facing inclusive dates. The example above
covers July 7, 8, and 9. Internally, `icalctl` converts the date-only end to
EventKit's exclusive next-midnight boundary, July 10 at 00:00 in this example.
For a one-day all-day event, pass the same date for both values, such as
`--start 2026-07-07 --end 2026-07-07 --all-day`.

When the user gives relative dates such as "today", "tomorrow", or "next Friday", resolve them to concrete dates before confirming the write.

## JSON Output

Every command accepts `--json` for compact machine-readable output. `completions` is normally used without `--json` because the useful output is the shell completion script.

Examples:

```sh
icalctl status --json
icalctl calendars --json
icalctl today --json
icalctl add "Meeting" --start 2026-07-07T09:00 --end 2026-07-07T09:30 --json
```

Use JSON for calendar-selection analysis, event id extraction, and scriptable workflows.
Event JSON includes calendar source/type/writability, normalized UTC and local
instants, duration, availability, and notes/URL presence. Detail and live write
responses include `alarm_count`; list responses use null when alarms were not
loaded. Dispatch on the top-level `type` and tolerate additive fields in the
documented schema-generation-1 contract.

## Commands

### `config`

Persistent configuration lives at `~/.icalctl/config.toml`. Initialize,
inspect, validate, or edit it with:

```sh
icalctl config path
icalctl config init
icalctl config show
icalctl config validate
icalctl config edit
```

Use generic dotted-key operations for changes:

```sh
icalctl config set flightaware.api_key
icalctl config set calendar.default_calendar_id CALENDAR_ID
icalctl config set reminders.default_list_id LIST_ID
icalctl config unset calendar.default_calendar_id
```

The API-key prompt does not echo. `--stdin` is available for secret input;
never pass a secret as a positional argument in an agent workflow. `config
show` and `config get flightaware.api_key` redact the secret. The config file
uses mode `0600`, writes are atomic, and an invalid `config edit` is rolled
back. Positional API-key values and insecure existing config permissions are
rejected.

For event and reminder adds, selection precedence is explicit CLI/JSON
selector, configured exact default ID, then the EventKit default. Configured
IDs are re-resolved and checked for writability; failure never silently falls
back. Update commands without a destination selector keep the current
calendar/list, and read commands do not inherit write defaults.

### `version`

Print semantic version and build provenance without requesting EventKit
permissions:

```sh
icalctl version
icalctl version --json
icalctl --version
```

### `status`

Print EventKit Calendar authorization status.

```sh
icalctl status
icalctl status --json
```

Use before read/write operations when permission state is unknown. If status is not `FullAccess`, do not assume event list or write operations will work.

### `doctor`

Diagnose Calendar permission and launch-context failures without requesting
access or changing Calendar data:

```sh
icalctl doctor
icalctl doctor --json
```

Use the reported `recommended_command` and remediation steps. The JSON includes
authorization status, process identity, terminal program, bundle identifier,
and embedded Calendar usage-description checks.

### `calendars`

List calendars available in Calendar.app.

```sh
icalctl calendars
icalctl calendars --json
icalctl calendars --source iCloud --writable-only --json
```

Use this before adding events so the agent can choose the best-fit calendar. The JSON form is the preferred source for exact calendar ids and source metadata.

When duplicate titles exist, title-only selection fails and prints the matching source, source id, calendar id, and writability. Use `--calendar-id`, or qualify a title with `--calendar-source` or `--source-id`.

The default target, when EventKit reports one, has `is_default_for_new_events: true`.
Use `--source <SOURCE>` for an exact source-title filter and
`--writable-only` when selecting destinations for writes.

### `default-calendar`

Show the EventKit default calendar for new events.

```sh
icalctl default-calendar
icalctl default-calendar --json
```

Use this before intentionally omitting all calendar selectors from `add`. The JSON output includes the calendar id, title, source, source id, type, writability, and `is_default_for_new_events: true`.

### `reminders`

Read local Apple Reminders with a dedicated command group:

```sh
icalctl reminders status --json
icalctl reminders lists --json
icalctl reminders lists --source iCloud --writable-only --json
icalctl reminders default-list --json
icalctl reminders list --json
icalctl reminders search "report" --json
icalctl reminders show <reminder-id-or-row> --json
```

`reminders list` and `reminders search` default to incomplete reminders. Pass
`--state completed` or `--state all` explicitly when needed. Use repeatable
`--list-id` selectors for exact automation. Repeatable `--list` selectors are
allowed, but duplicate titles fail with candidate ids, sources, source ids, and
writability; qualify a title with `--list-source` or `--source-id`.

Optional `--due-from` and `--due-to` filters exclude undated reminders. A
date-only upper bound includes the whole local day. JSON preserves date-only
due/start components as `kind: "date"`; timed values use `kind: "datetime"`
and report local components, timezone, normalized offset-bearing time, and UTC.

`reminders show` loads public alarm and recurrence details. Reminder priority
is reported separately as `none`, `high`, `medium`, or `low`; priority does not
imply an alarm.

Create reminders with:

```sh
icalctl reminders add "Submit report" --list-id LIST_ID \
  --due 2026-07-15 --priority high --dry-run --json
```

For a native subreminder, inspect the exact parent first, then pass its durable
id. If no list is supplied, the parent list is used; an explicit list must
match it:

```sh
icalctl reminders show PARENT_ID --json
icalctl reminders add "Child task" --parent-id PARENT_ID --dry-run --json
```

Confirm the parent title/id and target list in addition to the child fields.
Use `reminders update CHILD_ID --parent-id NEW_PARENT_ID --dry-run --json` to
reparent and `--clear-parent` to detach. Do not complete, move, or delete a
parent until its direct children have been handled; the CLI blocks those
potentially cascading mutations.

Omit `--due` for an undated reminder. Date-only due/start values remain true
date components. For timed values, precedence is explicit RFC3339 offset,
otherwise `--time-zone`, otherwise the Mac local timezone. `--time-zone` does
not change date-only values. Optional fields are `--start`, `--notes` or
`--notes-file`, `--url`, `--location`, and
`--priority none|low|medium|high`.
Provider-backed lists may normalize or drop free-text location, so inspect the
live JSON readback.

For timed due values, `--notify-at-due` requests a notification at the due
instant and repeatable `--notify-minutes-before N` requests positive-minute
early notifications. No notification is added by default. Notification flags
are invalid for undated or date-only reminders. Dry-run JSON reports the
computed absolute UTC and due-timezone instants. When notification flags are
supplied with `--if-exists update`, they replace existing alarms; omission
preserves them.

Use repeatable `--notify-at <RFC3339>` for arbitrary absolute alarms. Use the
complete `--geofence-title`, latitude, longitude, radius, and
`--geofence-proximity arrive|leave` tuple for one location alarm. Diagnose
Location state with `icalctl doctor --json`; do not treat authorization as a
prerequisite for constructing an explicit-coordinate EventKit geofence.
Simple recurrence uses `--repeat daily|weekly|monthly|yearly`, optional
`--repeat-interval`, and either `--repeat-count` or offset-bearing
`--repeat-until`; it requires a due or start date. On update, explicit schedule
options replace their existing collection, clear flags remove it, and omission
preserves it.

Duplicate identity is list id + parent id + title + due kind/value.
`--if-exists` defaults
to `error`; `skip` returns the existing reminder and `update` changes only
supplied non-identity fields. `--duplicate-window-seconds` applies only to
timed due matching. Date-only and undated identities stay exact. No alarms are
added unless a notification or geofence option is supplied.

Structured single-reminder JSON uses the same fields and validation as add
flags, including optional `parent_id`. Reminder batch JSON uses `version: 1`,
optional `defaults`, and a `reminders` array. Defaults may provide list
selection, parent id, timezone, priority, alarms, geofence, and recurrence.
Unknown fields, duplicate client ids, and
duplicate resolved identities fail preflight. Do not invent a second reminder
schema or bypass the normal add pipeline.
Inherited timezones apply only to timezone-less timed due/start values. A row
may set `time_zone`, `geofence`, or `recurrence` to `null` to clear that
default; omission inherits it. The batch pins each write to the exact list id
shown by preflight.

Use public EventKit for reminder fields except the isolated native hierarchy
bridge. Parent/child support uses private ReminderKit store, save-request, and
subtask-context APIs with runtime capability checks; treat it as macOS
version-sensitive and verify live readback. Do not inspect other private
metadata, use KVC, or edit the Calendar database. Flags, tags, sections,
attachments, templates, and messaging triggers remain outside the supported
boundary.

Event `show` loads public recurrence rules in addition to alarms. Inspect
`recurrence_count`, every entry in `recurrence_rules`, `is_detached`, and
`occurrence_date` before reasoning about a recurring event or exception. Event
weekday entries preserve both EventKit's weekday number and ordinal
`week_number`; do not collapse an ordinal rule such as first Monday into every
Monday. Event
recurrence creation is supported only through the normal `add` pipeline.
Structured JSON and batch rows accept the same normalized recurrence fields.
For a batch default, explicit row `recurrence: null` opts out. `--if-exists
skip` is safe only when the existing normalized rule is identical; a mismatch
must fail. For recurring update or delete, resolve one exact occurrence and
require the user to choose `--scope occurrence` or `--scope future`. Never infer
future scope from a series id or cached row.

### `today`

List today's events.

```sh
icalctl today
icalctl today --json
icalctl today --calendar "Work"
icalctl today --calendar "Work" --calendar "Personal" --json
icalctl today --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D --json
```

Use to answer "what is on my calendar today?" or to create fresh row-number references for events happening today. `--calendar` and `--calendar-id` can be passed more than once. Prefer ids for automation.

### `upcoming`

List upcoming events from now through N days from now.

```sh
icalctl upcoming
icalctl upcoming --days 7
icalctl upcoming --days 14 --calendar "Work" --json
```

Use for near-future agenda requests. `--days` defaults to `7`. Calendar filters can be passed more than once.

### `list`

List events in a bounded date range.

```sh
icalctl list --from 2026-07-07 --to 2026-07-07
icalctl list --from 2026-07-07T09:00 --to 2026-07-07T17:00
icalctl list --from 2026-07-07 --to 2026-07-14 --calendar "Work" --json
icalctl list --from 2026-07-07 --to 2026-07-14 --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D --json
```

Use when the user asks for events on a specific date or between two concrete times. `--calendar` and `--calendar-id` can be passed more than once to include only selected calendars.

### `search`

Search event title, notes, location, URL, and calendar name in a bounded range.

```sh
icalctl search "meeting" --from 2026-07-07 --to 2026-07-14
icalctl search "dentist" --from 2026-07-01 --to 2026-08-01 --json
icalctl search "sync" --from 2026-07-07 --to 2026-07-14 --calendar "Work"
```

Use for "find the event about..." requests. Search is case-insensitive. The command also refreshes the row cache.

### `show`

Show one event by exact EventKit identifier or cached row number.

```sh
icalctl show <event-id>
icalctl show <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00
icalctl show 1
icalctl show 1 --json
```

Use exact ids for durable series references. Use row numbers only after a fresh
list-like command in the same workflow. For a specific recurring occurrence,
use that fresh row or combine the series id with its exact RFC3339
`--occurrence-start`; EventKit can otherwise return the first occurrence.
For recurring events, inspect the rule collection, `is_detached`, and the
original `occurrence_date`.

### `add`

Create a calendar event.

```sh
icalctl add "Meeting" --start 2026-07-07T09:00 --end 2026-07-07T09:30
icalctl add "Meeting" --calendar "Work" --start 2026-07-07T09:00 --end 2026-07-07T09:30
icalctl add "Meeting" --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D --start 2026-07-07T09:00 --end 2026-07-07T09:30
icalctl add "Dentist" --calendar "Personal" --start 2026-07-07T15:00 --end 2026-07-07T16:00 --location "Clinic"
icalctl add "Focus block" --calendar "Work" --start 2026-07-07T13:00 --end 2026-07-07T15:00 --availability busy
icalctl add "Conference" --calendar "Travel" --start 2026-07-07 --end 2026-07-09 --all-day
```

Options:

- `<TITLE>`: required event title.
- `--start <START>`: required start date or datetime.
- `--end <END>`: required end date or datetime. Date-only values include the whole day.
- `-c, --calendar <CALENDAR>`: calendar title. If omitted, the configured exact default ID is used when present, otherwise EventKit's default calendar is used. Prefer choosing and confirming a calendar explicitly.
- `--calendar-id <CALENDAR_ID>`: exact EventKit calendar id. Preferred for automation.
- `--calendar-source <SOURCE> --calendar <CALENDAR>`: source-qualified calendar title.
- `--source-id <SOURCE_ID> --calendar <CALENDAR>`: source-id-qualified calendar title.
- `--notes <NOTES>`: event notes.
- `--notes-file <PATH>`: read exact UTF-8 notes from a file; use `-` for stdin. Conflicts with `--notes`.
- `--json-file <PATH>`: read a complete structured event draft; do not combine it with individual event fields.
- `--location <LOCATION>`: event location.
- `--url <URL>`: event URL.
- `--all-day`: mark the event as all-day.
- `--availability <AVAILABILITY>`: one of `busy`, `free`, `tentative`, or `unavailable`.
- `--time-zone <TZID>`: interpret timezone-less inputs in an IANA zone and store that zone on the event.
- `--alarm-minutes-before <MINUTES>`: add a display alarm before the event. Can be passed more than once.
- `--repeat <FREQUENCY>`: create a daily, weekly, monthly, or yearly series.
- `--repeat-interval <N>`: positive interval, defaulting to 1.
- `--repeat-weekday <DAY>`: repeatable weekday constraint.
- `--repeat-month-day <N>`: repeatable positive or negative constraint for monthly rules only.
- `--repeat-count <N>` or `--repeat-until <RFC3339>`: optional exclusive termination choice.
- `--if-exists <POLICY>`: `error` (default), `skip`, or `update` for a matching calendar/title/start/end/all-day identity.
- `--duplicate-window-seconds <N>`: optional start/end tolerance; defaults to exact matching (`0`).
- `--dry-run`: validate and print the resolved event draft without writing.
- `--json`: print the created event as JSON.

The event JSON has `calendar_selection: "explicit"` when a calendar selector was passed and `calendar_selection: "eventkit_default"` when EventKit's default was used. It reports `write_action` as `created`, `skipped`, or `updated`. Prefer `--if-exists skip` for retry-safe writes; preview it first because a duplicate-window tolerance can match a nearby event. For recurrence, skip requires an identical normalized rule. Recurring `--if-exists update` remains blocked because add/reconcile has no exact occurrence target; use the explicit `update` command instead.

JSON draft files require `title`, `start`, and `end` and support `calendar`,
`calendar_id`, `calendar_source`, `source_id`, `notes`, `location`, `url`,
`availability`, `time_zone`, `alarm_minutes_before`, `all_day`, `timed`, and
nested `recurrence`.
Keep `--if-exists`, `--duplicate-window-seconds`, `--dry-run`, and `--json` on
the command. Always dry-run a JSON draft and inspect its resolved calendar and
times before confirmation.

Never run `add` until the calendar choice and final event details have been confirmed by the user.
For recurrence, the confirmation must include the normalized frequency,
interval, weekday/month-day constraints, and count/end date. Dry-run the full
series definition before creating it. For a timed series that should keep the
same wall-clock time across daylight-saving changes, require its IANA
`--time-zone`; an offset alone does not identify future timezone transitions.
Recurring all-day events must use date-only `--start` and `--end` values.

### `batch add`

Validate and import a versioned JSON event batch:

```sh
icalctl batch add --file events.json --if-exists skip --dry-run --json
icalctl batch add --file events.json --if-exists skip --json
```

The file contains `version: 1`, optional `defaults`, and an `events` array.
Every event requires `title`, `start`, and `end`; prefer exact `calendar_id`
values in defaults or individual entries. Datetime and timezone behavior is the
same as `add`. If a row inherits or sets both recurrence and `all_day: true`,
its `start` and `end` must be date-only.

Always inspect the complete dry-run result and obtain confirmation for every
planned create or update before running the live command. `--if-exists error`
is the default. Use `skip` for idempotent reruns. Use `update` only when the
user has confirmed the optional-field patches; supplied alarm arrays replace
existing alarms. By default any preflight error blocks all writes.
For recurring matches, `skip` requires the identical normalized rule and
batch `update` is rejected because a batch match has no exact occurrence and
scope.
`--continue-on-error` permits partial imports and must be disclosed before
confirmation because EventKit cannot roll back earlier successful writes.

### `travel flight`

Format one flight leg and pass it through normal event creation:

```sh
icalctl travel flight HO1607 \
  --from PVG --to HEL \
  --departure 2026-07-11T09:25:00+08:00 \
  --arrival 2026-07-11T14:00:00+03:00 \
  --calendar-id CALENDAR_ID \
  --if-exists skip --dry-run --json
```

Departure and arrival must include explicit RFC3339 offsets. Never infer an
offset from an airport code. The helper uppercases the flight number and 3-4
letter airport codes, generates title/location/route notes, defaults to busy
with no alarms, and deliberately stores no single EventKit timezone. Extra
`--notes` or `--notes-file` content is appended after one blank line.

All ordinary calendar selectors, URL, availability, alarm, duplicate, and
dry-run safeguards still apply. One invocation represents one leg. Use the
documented generic batch JSON recipe for multi-leg itineraries; there is no
travel-specific batch schema. Event creation does not call an airline provider.

### `travel serve`

Serve the read-only local travel atlas:

```sh
icalctl travel serve
icalctl travel serve --calendar-id EXACT_EVENTKIT_ID
```

This command reads future canonical `travel flight` events through EventKit,
serves the bundled MapLibre interface on a loopback address, and never mutates
Calendar. No write confirmation is needed. When filtering, first verify exact
ids with `icalctl calendars --json`; repeat `--calendar-id` for multiple
calendars. If no CLI ids are supplied, `travel.calendar_ids` applies, and an
empty configured list reads all calendars.

The printed URL contains a random per-launch capability that remains valid
until that server process stops and establishes a protected local browser
session. Do not share or record that URL. The default request starts at the
current instant for the configured upcoming window. The page's date controls
use inclusive date-only bounds and allow at most 366 days.

FlightAware is optional and fail-open. It uses only
`flightaware.api_key` from `~/.icalctl/config.toml`; never ask for or pass an API
key on the command line. Missing keys, provider failures, ambiguous matches,
backoff, or quota exhaustion leave Calendar-only legs with warnings. Normalized
provider cache and usage state normally live under
`~/Library/Caches/icalctl/flightaware/` (`XDG_CACHE_HOME` overrides the cache
root). The key and raw provider responses must never be surfaced or persisted.

Map/globe switching reuses the current response. The default basemap is
OpenFreeMap's Bright street style. `travel.map.style_url` is sent to the local
browser, so it must not contain secrets. Unknown airports and provider warnings
remain visible without dropping an otherwise valid leg.

### `update`

Update an event by exact EventKit identifier or cached row number.

```sh
icalctl update <event-id> --title "New title"
icalctl update 1 --location "Library"
icalctl update 1 --calendar "Work"
icalctl update 1 --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D
icalctl update 1 --start 2026-07-07T10:00 --end 2026-07-07T10:30
icalctl update 1 --clear-location --clear-notes --clear-url
icalctl update 1 --add-alarm-minutes-before 30
icalctl update <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00 --scope occurrence --location "Library" --dry-run --json
icalctl update <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00 --scope future --location "Library" --dry-run --json
icalctl update 1 --json
```

Options:

- `<ID>`: exact EventKit event identifier, or row number from the last event list.
- `--occurrence-start <RFC3339>`: exact selected start when using a recurring series id.
- `--scope <SCOPE>`: required for recurring targets; `occurrence` changes only the selected occurrence, while `future` changes it and later occurrences.
- `--title <TITLE>`: replace the title.
- `--start <START>`: replace the start date or datetime.
- `--end <END>`: replace the end date or datetime.
- `-c, --calendar <CALENDAR>`: move the event to another calendar by title.
- `--calendar-id <CALENDAR_ID>`: move the event to an exact EventKit calendar id.
- `--calendar-source <SOURCE> --calendar <CALENDAR>`: move by a source-qualified title.
- `--source-id <SOURCE_ID> --calendar <CALENDAR>`: move by a source-id-qualified title.
- `--notes <NOTES>`: replace notes.
- `--clear-notes`: clear notes.
- `--location <LOCATION>`: replace location.
- `--clear-location`: clear location.
- `--url <URL>`: replace URL.
- `--clear-url`: clear URL.
- `--all-day`: mark as all-day.
- `--timed`: mark as timed.
- `--availability <AVAILABILITY>`: one of `busy`, `free`, `tentative`, or `unavailable`.
- `--time-zone <TZID>`: interpret timezone-less updated times in an IANA zone and store that zone.
- `--clear-time-zone`: clear the stored event timezone.
- `--add-alarm-minutes-before <MINUTES>`: add a display alarm before the event. Can be passed more than once.
- `--dry-run`: validate and print the resulting event draft without writing.
- `--json`: print the updated event as JSON.

Before updating, confirm the exact event and the intended changes. For a
recurring target, confirm both the exact occurrence and whether the user means
only that occurrence or that occurrence and all future ones; never choose scope
for the user. A fresh cached row supplies the occurrence start but does not
supply scope. If moving to a different calendar, inspect calendars and confirm
the destination calendar.

### `delete`

Delete an event by exact EventKit identifier or cached row number.

```sh
icalctl delete <event-id>
icalctl delete 1
icalctl delete 1 --force
icalctl delete 1 --force --json
icalctl delete <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00 --scope occurrence
icalctl delete <recurring-event-id> --occurrence-start 2026-07-20T09:00:00+03:00 --scope future
```

By default, the command prompts for the word `delete`. `--force` skips the interactive prompt and is intended for scripts. Even when using `--force`, an agent must get explicit user confirmation before deleting. Recurring deletion also requires an exact occurrence and explicit `--scope occurrence|future`; confirm both with the user. A cached row identifies an occurrence but never authorizes future deletion.

### `completions`

Print a shell completion script to stdout.

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

Then ensure `~/.zfunc` is in `fpath` before `compinit` in the user's zsh config.

## Row Cache

List-like commands write a cache that maps displayed row numbers to EventKit ids:

```sh
icalctl today
icalctl upcoming --days 7
icalctl list --from 2026-07-07 --to 2026-07-07
icalctl search "meeting" --from 2026-07-07 --to 2026-07-14
```

The cache is stored at:

```text
~/Library/Caches/icalctl/last-events.json
```

or, if `XDG_CACHE_HOME` is set:

```text
$XDG_CACHE_HOME/icalctl/last-events.json
```

The cache stores only the most recent event rows needed to resolve row numbers for `show`, `update`, and `delete`. Calendar data remains in EventKit. If a row reference is stale or missing, rerun the relevant list/search command.

Reminder rows are stored separately at:

```text
~/Library/Caches/icalctl/last-reminders.json
```

or `$XDG_CACHE_HOME/icalctl/last-reminders.json`. Only `reminders list` and
`reminders search` refresh it. `reminders show` consumes reminder rows; add
does not accept a row target.

## Safe Examples

Read today's events:

```sh
icalctl today --json
```

Find an event, inspect it, then update it after user confirmation:

```sh
icalctl search "project sync" --from 2026-07-07 --to 2026-07-14 --json
icalctl show 1 --json
icalctl update 1 --location "Room 3" --dry-run --json
icalctl update 1 --location "Room 3" --json
```

Add an event after calendar analysis and confirmation:

```sh
icalctl calendars --json
icalctl add "Dentist" --calendar-id CALENDAR_ID --start 2026-07-07T15:00 --end 2026-07-07T16:00 --location "Clinic" --json
```

Delete after confirming the exact event with the user:

```sh
icalctl show 1 --json
icalctl delete 1 --force --json
```
