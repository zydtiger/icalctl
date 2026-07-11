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
- Use row numbers only immediately after a fresh `today`, `upcoming`, `list`, or `search`; otherwise use the exact EventKit id or rerun the list command.
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

Duplicate identity is list id + title + due kind/value. `--if-exists` defaults
to `error`; `skip` returns the existing reminder and `update` changes only
supplied non-identity fields. `--duplicate-window-seconds` applies only to
timed due matching. Date-only and undated identities stay exact. No alarms are
added unless a notification or geofence option is supplied.

Only use the public EventKit reminder surface. Do not inspect private selectors,
use KVC for Reminders.app-only metadata, or edit the Calendar database. Flags,
tags, sections, subtasks, attachments, templates, and messaging triggers are
outside the public boundary.

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
icalctl show 1
icalctl show 1 --json
```

Use exact ids for durable references. Use row numbers only after a fresh list-like command in the same workflow.

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
- `-c, --calendar <CALENDAR>`: calendar title. If omitted, EventKit uses the system default calendar for new events. Prefer choosing and confirming a calendar explicitly.
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
- `--if-exists <POLICY>`: `error` (default), `skip`, or `update` for a matching calendar/title/start/end/all-day identity.
- `--duplicate-window-seconds <N>`: optional start/end tolerance; defaults to exact matching (`0`).
- `--dry-run`: validate and print the resolved event draft without writing.
- `--json`: print the created event as JSON.

The event JSON has `calendar_selection: "explicit"` when a calendar selector was passed and `calendar_selection: "eventkit_default"` when EventKit's default was used. It reports `write_action` as `created`, `skipped`, or `updated`. Prefer `--if-exists skip` for retry-safe agent writes; preview it first because a duplicate-window tolerance can match a nearby event.

JSON draft files require `title`, `start`, and `end` and support `calendar`,
`calendar_id`, `calendar_source`, `source_id`, `notes`, `location`, `url`,
`availability`, `time_zone`, `alarm_minutes_before`, `all_day`, and `timed`.
Keep `--if-exists`, `--duplicate-window-seconds`, `--dry-run`, and `--json` on
the command. Always dry-run a JSON draft and inspect its resolved calendar and
times before confirmation.

Never run `add` until the calendar choice and final event details have been confirmed by the user.

### `batch add`

Validate and import a versioned JSON event batch:

```sh
icalctl batch add --file events.json --if-exists skip --dry-run --json
icalctl batch add --file events.json --if-exists skip --json
```

The file contains `version: 1`, optional `defaults`, and an `events` array.
Every event requires `title`, `start`, and `end`; prefer exact `calendar_id`
values in defaults or individual entries. Datetime and timezone behavior is the
same as `add`.

Always inspect the complete dry-run result and obtain confirmation for every
planned create or update before running the live command. `--if-exists error`
is the default. Use `skip` for idempotent reruns. Use `update` only when the
user has confirmed the optional-field patches; supplied alarm arrays replace
existing alarms. By default any preflight error blocks all writes.
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
travel-specific batch schema or airline-data lookup.

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
icalctl update 1 --json
```

Options:

- `<ID>`: exact EventKit event identifier, or row number from the last event list.
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

Before updating, confirm the exact event and the intended changes. If moving to a different calendar, inspect calendars and confirm the destination calendar.

### `delete`

Delete an event by exact EventKit identifier or cached row number.

```sh
icalctl delete <event-id>
icalctl delete 1
icalctl delete 1 --force
icalctl delete 1 --force --json
```

By default, the command prompts for the word `delete`. `--force` skips the interactive prompt and is intended for scripts. Even when using `--force`, an agent must get explicit user confirmation before deleting.

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
