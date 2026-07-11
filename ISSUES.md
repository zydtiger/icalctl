# icalctl Issues

These are improvement notes from using `icalctl` to add a multi-flight itinerary to the local Apple Calendar store on 2026-07-08.

## 1. Support stable calendar targeting by id/source

**Status: Resolved.** Implemented exact calendar-id targeting, source-qualified title selectors, duplicate-title errors, stable read filtering, and calendar source provenance in event output.

Problem: `add` and `update --calendar` target calendars by title only. In the observed calendar store, there were two writable calendars named `Calendar`: one under Exchange and one under iCloud. The current implementation uses the first title match, so `--calendar "Calendar"` could write to the wrong account.

Proposed changes:

- Add `--calendar-id <EVENTKIT_CALENDAR_ID>` to `add`, `update`, `list`, `today`, `upcoming`, and `search`.
- Add source-qualified selectors such as `--calendar-source iCloud --calendar Calendar` or `--source-id <SOURCE_ID> --calendar Calendar`.
- Make title-only `--calendar` fail when multiple calendars share the same title. The error should print the matching title, source, source id, calendar id, and writability for each candidate.
- Keep title-only selection when the title is unique.
- Prefer exact calendar ids in machine workflows and examples.

Acceptance checks:

- With duplicate `Calendar` titles across iCloud and Exchange, `icalctl add ... --calendar Calendar` exits non-zero and lists both candidates.
- `icalctl add ... --calendar-id A46E7273-2813-48A6-8F74-67B9E9E3D55D` writes to that exact calendar.
- JSON output for created/updated events includes `calendar`, `calendar_id`, `calendar_source`, and `calendar_source_id`.

## 2. Show and expose the default new-event calendar

**Status: Resolved.** Added default-calendar reporting, default markers in calendar lists, and explicit-versus-EventKit-default provenance in add output.

Problem: omitting `--calendar` uses EventKit's default calendar, but users and agents cannot see that target from `icalctl` before writing. In the observed setup, the default new-event calendar was `Tutor` on iCloud, not the requested standard iCloud `Calendar`.

Proposed changes:

- Add `icalctl default-calendar` with human and JSON output.
- Add `is_default_for_new_events` to `calendars --json`.
- In human `calendars` output, mark the default calendar clearly.
- In `add --json`, include whether the target calendar came from an explicit selector or EventKit default.

Acceptance checks:

- `icalctl default-calendar --json` returns id, title, source, source id, calendar type, and writability.
- `icalctl calendars` marks exactly one writable default calendar when EventKit reports one.

## 3. Add dry-run planning for writes

**Status: Resolved.** Added non-mutating add/update plans with shared permission, calendar, date-range, URL, availability, and alarm validation plus exact duplicate warnings.

Problem: live writes are high-risk, especially when date parsing, default calendar selection, or duplicate calendar titles are involved.

Proposed changes:

- Add `--dry-run` to `add` and `update`.
- Dry-run should perform permission checks, calendar resolution, datetime parsing, URL validation, availability validation, and duplicate warnings, but should not call EventKit save/update.
- JSON dry-run output should show the resolved calendar id/source, normalized start/end instants, all-day/timed state, availability, alarm count, and notes/location/url presence.

Acceptance checks:

- `icalctl add ... --dry-run --json` returns the event draft and `would_write: false`.
- Dry-run catches ambiguous calendars and invalid date ranges the same way live write does.

## 4. Improve timezone handling and reporting

**Status: Resolved.** Preserved offset-bearing write inputs, made `--time-zone` control naive input parsing and EventKit storage, added UTC/local/event-timezone and duration fields, included offsets in human output, and covered timezone-crossing routes.

Problem: offset datetimes are accepted, but EventKit stores absolute instants and output may be rendered in the machine's local timezone. For travel, users need confidence that airport-local times and offsets were interpreted correctly.

Proposed changes:

- Preserve the original input strings in JSON write output when possible, alongside normalized ISO UTC/local representations.
- Add optional `--time-zone <TZID>` for event display timezone where EventKit supports it.
- Consider separate `--start-time-zone` and `--end-time-zone` metadata for travel-style events where departure and arrival are in different zones. If EventKit cannot store both, include a first-class notes helper or structured JSON echo.
- Include timezone offset in human event output, not only local clock time.
- Add tests for flights crossing timezone boundaries, including same displayed local clock time with different offsets.

Acceptance checks:

- Creating `2026-07-12T15:55:00+03:00` to `2026-07-12T15:55:00+02:00` is accepted as a one-hour event and the output makes that clear.
- Read-back output can show the event in UTC and in the requested/source timezone.

## 5. Add idempotent batch creation

**Status: Resolved.** Added versioned JSON batch imports with full preflight, dry-run plans, exact duplicate policies, patch-style updates, alarm replacement, per-item results, and explicit partial-failure handling.

Problem: adding six related flight events required repeated writes or an external script. There is no built-in way to create a batch safely, preview it, and skip duplicates.

Proposed changes:

- Add `icalctl batch add --file events.json` or `icalctl import-json`.
- Support an idempotency key or duplicate policy: `--if-exists skip|update|error`.
- Duplicate matching should support at least title + start + end + calendar id.
- Batch output should summarize created, skipped, updated, and failed items with event ids.
- Ideally support transactional behavior when practical: either `--continue-on-error` or fail before writing if validation errors are found.

Acceptance checks:

- Re-running the same batch with `--if-exists skip` creates zero duplicate events.
- JSON output reports per-item status and event id.

## 6. Make permission bootstrap more diagnosable

**Status: Resolved.** Added parseable `doctor` diagnostics for authorization, process/launch context, embedded privacy metadata, and next steps; permission failures now include current status and targeted Terminal/Mach remediation.

Problem: from an embedded/sandboxed tool environment, `icalctl status --json` returned `NotDetermined`, and `icalctl calendars --json` failed with an EventKit/Mach authorization error. Running outside the sandbox succeeded. The error should point users directly to the right fix.

Proposed changes:

- Add `icalctl doctor` or `icalctl permissions` to report authorization status, process identity, bundle/plist status, and a recommended next command.
- When authorization request fails with `NSMachErrorDomain` / Mach error 4099, print a targeted message: run `icalctl calendars` from Terminal.app, iTerm, or Ghostty and approve the Calendar prompt.
- Document how to reset Calendar permission for the binary if needed.
- Consider a `--request-access` command that exists only to trigger and explain the macOS prompt.

Acceptance checks:

- Permission errors include the current authorization status and a specific remediation.
- `icalctl doctor --json` is parseable by agents.

## 7. Safer calendar filtering for reads

**Status: Resolved.** Read commands reuse exact id/source-aware calendar resolution, reject ambiguous titles with candidate details, accept multiple calendar ids, and calendar discovery now supports exact `--source` and `--writable-only` filters.

Problem: read commands also filter calendars by title only. Duplicate titles can make `list/search/today/upcoming --calendar Calendar` include the wrong calendar or multiple unintended calendars, depending on the EventKit wrapper behavior.

Proposed changes:

- Reuse the stable selector work from issue 1 for all read filters.
- Allow multiple `--calendar-id` values.
- Make ambiguous title filters fail unless the user explicitly asks for all title matches.
- Add `--source` and `--writable-only` filters to `calendars`.

Acceptance checks:

- `icalctl list --calendar Calendar` fails with duplicate-title candidates.
- `icalctl list --calendar-id <id>` only returns events from that exact calendar.

## 8. Improve event output schema for automation

**Status: Resolved.** Event JSON now exposes calendar account/type/writability provenance, normalized UTC/local/event-timezone instants, duration, availability, notes/URL presence, and loaded alarm counts; the schema stability contract is documented.

Problem: event JSON is useful, but agents need more provenance and normalized time fields to verify writes without calling Swift/EventKit directly.

Proposed changes:

- Include `calendar_source`, `calendar_source_id`, `calendar_type`, and `allows_calendar_modifications` in `EventReport`.
- Include normalized `start_utc`, `end_utc`, `start_local`, `end_local`, and maybe `duration_seconds`.
- Include `has_notes`, `has_url`, `alarm_count`, and availability in add/update responses.
- Version the JSON schema or document stability expectations.

Acceptance checks:

- A created event response has enough fields to confirm exact calendar account, exact calendar id, exact instants, and alarm count.

## 9. Add duplicate detection to single-event add

**Status: Resolved.** Single-event add now defaults to duplicate errors, supports `skip` and patch-style `update`, offers an explicit start/end tolerance window, reports the resulting write action/id, and previews matches without writing.

Problem: single `add` always creates a new event. For agent workflows, accidental retries can create duplicate calendar entries.

Proposed changes:

- Add `--if-exists skip|error|update` to `add`.
- Add `--duplicate-window-seconds <N>` with a sensible default for exact title/start/end/calendar matching.
- In dry-run mode, warn about likely duplicates.

Acceptance checks:

- Re-running the same `add` with `--if-exists skip --json` returns the existing event id and does not create a duplicate.

## 10. Improve shell ergonomics for long notes and structured event details

**Status: Resolved.** Add now reads exact UTF-8 notes from files or stdin and accepts strict, complete JSON event drafts covering calendar selection, content, availability, alarms, timezone, and all-day/timed state with full dry-run validation.

Problem: passing multi-line notes through shell arguments is awkward and error-prone.

Proposed changes:

- Add `--notes-file <PATH>` and maybe `--json-file <PATH>` for event draft input.
- Support reading notes from stdin with `--notes-file -`.
- For JSON draft input, support fields for title, calendar selector, start, end, location, notes, url, availability, alarms, all-day/timed.

Acceptance checks:

- `icalctl add ... --notes-file notes.txt` creates an event with exact note contents.
- `icalctl add --json-file event.json --dry-run --json` validates without writing.

## 11. Add integration tests around risky EventKit behavior

**Status: Resolved.** Selector and datetime unit matrices now cover unique/ambiguous/missing/read-only calendars, ids and source qualifiers, offset-crossing durations, all-day ranges, and timezone edge cases; an ignored opt-in suite verifies real permission/default state and guarded create/read-back/delete behavior on an exact `icalctl Test` calendar.

Problem: the riskiest behavior depends on real EventKit state: duplicate calendar titles, default calendar selection, permissions, timezone parsing, and read-back.

Proposed changes:

- Add unit tests for calendar selector resolution using mocked calendar lists.
- Add tests for ambiguous duplicate titles.
- Add tests for offset datetime parsing and duration calculation.
- Add an ignored/manual integration test suite for real macOS EventKit writes against a temporary or explicitly named test calendar.

Acceptance checks:

- Selector resolution tests cover unique title, duplicate title, id, source-qualified title, missing calendar, and read-only calendar.
- Date parsing tests cover offset-crossing routes and all-day ranges.

## 12. Consider first-class travel helpers later

**Status: Resolved.** Added a thin deterministic `travel flight` formatter that validates offset-bearing flight legs and routes the generated title, route, notes, busy/default-alarm behavior, calendar selection, duplicate policy, dry-run, and write through the existing single-event add pipeline; multi-leg travel remains ordinary batch JSON.

Problem: travel events often need structured route, carrier/flight number, departure/arrival airport local times, and no alarms by default. This is outside the core calendar CRUD API, but it is a common agent use case.

Proposed changes:

- Keep the core API generic first.
- Later, consider a small `icalctl travel flight` helper or documented JSON recipe that maps flight fields into title/location/notes consistently.
- Do not add this before stable calendar targeting, dry-run, and batch support.

Acceptance checks:

- A travel helper, if added, is just a thin convenience wrapper over the generic add/batch path.

## 13. Add first-class Apple Reminders support

**Status: Resolved.** All five phases are complete: public EventKit reminder
reads, safe creation and lifecycle writes, advanced scheduling, strict JSON and
batch input, guarded integration coverage, and agent confirmation safeguards.

Implementation phases:

- [x] Phase 1: read-only authorization, list discovery/default, list/search/show,
  due/state filters, public alarm/recurrence detail, diagnostics, and a separate
  reminder row cache.
- [x] Phase 2: safe basic creation with exact-list targeting, dry-run, duplicate
  handling, date-only/timed due values, priority, timed-due notifications, and
  agent confirmation rules.
- [x] Phase 3: update, completion/uncompletion, list moves, and delete.
- [x] Phase 4: absolute alarms, recurrence, geofences, and Location diagnostics.
- [x] Phase 5: structured JSON input, batch creation, guarded integration tests,
  documentation hardening, and final Issue 13 resolution.

Problem: `icalctl` only manages calendar events even though EventKit also exposes
the user's Reminders lists and reminder items. Agents need to create a reminder
that is due on a date, or due at a specific date and time, without turning the
task into a fake calendar event. The current CLI also cannot express an
high-priority reminder or attach reminder alarms.

Priority and alarms are separate EventKit concepts. Priority maps
to reminder priority (`none`, `low`, `medium`, or `high`); alarms control when a
notification or location trigger fires. A reminder must be able to use either
feature independently or both together.

Proposed changes:

- Add a first-class `reminders` command group with reminder-specific `status`,
  `lists`, `list`, `search`, `show`, `add`, `update`, `complete`, `uncomplete`,
  and `delete` commands. Keep event commands backward compatible.
- Request and diagnose Reminders authorization separately from Calendar access,
  and embed `NSRemindersFullAccessUsageDescription` plus the legacy
  `NSRemindersUsageDescription` fallback in `Info.plist`.
- Target reminder lists safely by exact EventKit id, unique title, or
  source-qualified title. Reject ambiguous and read-only list selections, show
  the default reminder list, and apply the same inspect/dry-run/confirm safety
  rules used for event writes.
- Support reminders with no due date, a date-only due value, or a due datetime.
  Preserve the distinction in human and JSON output and support an explicit IANA
  timezone for timed due values. Date-only reminders must remain all-day rather
  than being silently converted to midnight.
- Support EventKit reminder fields: title, notes, URL, location, start date,
  due date, priority, completion state/date, multiple alarms, and the single
  recurrence rule EventKit allows. Expose priority as
  `none|low|medium|high` through `--priority`; do not add a second importance
  flag.
- Support time alarms at the due instant, relative early alarms, and absolute
  alarm datetimes. Support arrival/departure geofence alarms when coordinates,
  radius, and proximity are supplied, including the separate Location permission
  and clear error reporting. Do not imply that priority itself creates an alarm.
- Add dry-run plans, exact duplicate handling, structured JSON input, batch
  creation, update/clear semantics, and JSON reports equivalent in safety and
  provenance to event workflows. Reminder row references must use a separate
  cache so an event list cannot accidentally resolve a reminder mutation.
- Document the public EventKit boundary. Reminders.app-only features that EventKit
  does not publicly expose, such as flags, tags, sections, subtasks, attachments,
  templates, and messaging triggers, are out of scope unless Apple adds public
  APIs; do not use private selectors or edit the Calendar database directly.

Suggested CLI examples:

```sh
icalctl reminders add "Submit report" \
  --list-id LIST_ID \
  --due 2026-07-15 \
  --priority high \
  --dry-run --json

icalctl reminders add "Call the dentist" \
  --list-id LIST_ID \
  --due 2026-07-15T14:30 \
  --time-zone Europe/Helsinki \
  --notify-at-due \
  --notify-minutes-before 30 \
  --json
```

Acceptance checks:

- Creating with `--due 2026-07-15` reads back as a date-only/all-day reminder;
  creating with `--due 2026-07-15T14:30 --time-zone Europe/Helsinki` reads back
  with the same local due time and normalized instant.
- `--priority high` creates a high-priority reminder without adding an alarm
  unless a notification option is also supplied.
- `--notify-at-due` produces an EventKit display alarm at the timed due value;
  repeatable `--notify-minutes-before` values produce deterministic absolute
  alarms that round-trip in `show --json`.
- Complete/uncomplete preserves an explicit completion timestamp correctly, and
  update can clear due/start dates, priority, recurrence, location, URL, notes,
  and alarms without changing omitted fields.
- Duplicate reminder-list titles fail with stable candidate ids and sources;
  exact `--list-id` always selects the intended writable list.
- Dry-run performs permission, list, date/timezone, priority, recurrence, URL,
  location, and alarm validation without calling EventKit save APIs.
- Unit tests cover date-only versus timed due components, priority mapping,
  alarm validation, recurrence serialization, selector ambiguity, and cache
  separation. An ignored guarded integration test creates, reads, completes,
  uncompletes, and deletes a reminder only in an exact `icalctl Test` list.

## 14. Add recurring calendar-event support later

**Status: In progress.** Phases 1 through 4 are complete: recurrence is readable,
new series can be created, structured/batch duplicate workflows understand
normalized rules, and recurring update/delete require exact occurrence and
explicit occurrence-versus-future scope. DST/all-day hardening remains.

Implementation phases:

- [x] Phase 1: read-only recurrence rules, termination conditions, and detached
  occurrence/original-date reporting in event details.
- [x] Phase 2: recurring creation for daily, weekly, monthly, and yearly rules.
- [x] Phase 3: recurrence-aware structured JSON, batch, and duplicate handling.
- [x] Phase 4: explicit one-occurrence and future-occurrence update/delete scope.
- [ ] Phase 5: DST/all-day correctness, batch compatibility, guarded EventKit
  integration coverage, documentation hardening, and final resolution.

Remaining work is DST/all-day, batch integration, and guarded EventKit
hardening.

Proposed changes:

- Show recurrence rules and exception status in human and JSON event details.
- Add recurring event creation for daily, weekly, monthly, and yearly rules,
  including interval, selected weekdays or month days, and optional end date or
  occurrence count.
- Add update and delete scope controls for one occurrence versus future
  occurrences. Never infer a series-wide mutation from a cached row number.
- Preserve all-day behavior, timezone metadata, and local wall-clock time across
  daylight-saving transitions. Validate unsupported provider combinations before
  writing and include the resolved recurrence rule in dry-run output.
- Keep recurrence on the existing event add/update/delete paths so calendar
  selection, duplicate checks, confirmation, JSON reporting, and batch safety
  remain consistent.

Acceptance checks:

- Daily, weekday-only weekly, monthly, and yearly rules round-trip through
  EventKit and `show --json` without losing their interval or end condition.
- Dry-run reports the normalized rule and performs no write.
- Updating or deleting one occurrence leaves the rest of the series intact;
  future-occurrence scope changes only the selected occurrence and later ones.
- Timed recurrences keep the intended local clock time across DST changes, and
  all-day recurrences remain date-only.
