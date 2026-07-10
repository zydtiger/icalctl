# icalctl

`icalctl` is a macOS command-line interface for the local Apple Calendar store.
It talks to Apple's EventKit framework from Rust, so it works with the calendars
already configured in Calendar.app, including iCloud, Google, Exchange, and
local calendars.

The command is intended for fast terminal workflows and scripts. Human-readable
output is the default; pass `--json` to any command for compact JSON.

## Status

This is an early Apple Silicon macOS CLI. It currently supports:

- listing calendars
- listing, showing, and searching events
- creating, updating, and deleting events
- compact JSON output
- cached row-number references for the most recent event list
- shell completion generation

It does not yet implement ICS import/export, recurrence editing UX, invite/RSVP
workflows, or Homebrew packaging.

## Requirements

- macOS
- Apple Calendar configured with the accounts you want to access
- Rust toolchain for local builds
- Calendar permission granted through the macOS privacy prompt

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

## Permissions

`icalctl` embeds the Calendar privacy usage strings in the Mach-O binary through
`embed_plist`. On first real Calendar access, macOS should ask for Calendar
permission.

Run the first permission test from a normal terminal such as Terminal.app,
iTerm, or Ghostty:

```sh
icalctl calendars
```

If permission was granted, this should report your calendars. If the command was
launched from an embedded tool environment, macOS may attribute the permission
request to that parent process instead of the terminal.

Check current authorization status:

```sh
icalctl status
icalctl status --json
```

## Commands

```sh
icalctl status
icalctl calendars
icalctl default-calendar
icalctl today
icalctl upcoming --days 7
icalctl list --from 2026-07-07 --to 2026-07-07
icalctl search meeting --from 2026-07-07 --to 2026-07-14
icalctl show <event-id-or-row>
icalctl add "Meeting" --start 2026-07-07T09:00 --end 2026-07-07T09:30
icalctl update <event-id-or-row> --location "Library"
icalctl delete <event-id-or-row>
```

Use command-specific help for the full option set:

```sh
icalctl add --help
icalctl update --help
icalctl delete --help
```

## Date Input

Date-only values use the local timezone.

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
`calendar_source_id`. A successful `add --json` also reports
`calendar_selection` as `explicit` or `eventkit_default`.

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

## Deleting Events

By default, delete asks for confirmation:

```sh
icalctl delete 1
```

Type `delete` when prompted. Use `--force` for scripts:

```sh
icalctl delete 1 --force
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
icalctl completions zsh
```

## Architecture

```text
clap CLI
  -> command dispatcher
  -> EventKit adapter via eventkit-rs
  -> macOS Calendar store
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

`eventkit-rs` is the high-level wrapper. If a future feature needs lower-level
EventKit access, the likely escape hatch is `objc2-event-kit`.
