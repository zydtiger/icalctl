# Architecture

`icalctl` is organized around Calendar, Reminders, travel, configuration, and
presentation domains. Command-line parsing is an entry point, and macOS or HTTP
integrations are adapters at the edges.

```text
CLI parsing and dispatch
          |
          v
Calendar  Reminders  Travel  Configuration
    |         |        |          |
    v         v        v          v
EventKit  ReminderKit  Flight provider  Filesystem
                      and web server
          |
          v
      Models and output
```

The intended source layout is:

```text
src/
  cli/                 command definitions and dispatch
  calendar/            selection, reads, recurrence, mutations, and adapters
  reminders/           selection, queries, scheduling, mutations, and adapters
  travel/
    model.rs           provider-neutral itinerary and live-status models
    flight.rs          canonical flight-note formatting
    parser.rs          canonical note parsing
    collection.rs      event conversion, warnings, sorting, and deduplication
    airports.rs        bundled airport metadata
    service.rs         Calendar discovery and optional enrichment orchestration
    server/
      security.rs      capability and request-origin validation
      routing.rs       method and path dispatch
      api.rs           versioned response construction
      range.rs         strict inclusive date-range parsing
      assets.rs        embedded frontend delivery and content types
  flightaware/         FlightAware adapter over provider-neutral travel models
    client.rs          HTTP transport and provider response types
    matching.rs        deterministic flight candidate selection
    normalization.rs   provider-to-domain status and position conversion
    cache.rs           normalized cache persistence and stale fallback
    quota.rs           cross-process quota accounting and backoff
  config/              schema, persistence, validation, and CLI operations
  models/              shared serialization and report types
  output/              human-readable rendering
```

Within Calendar, selection, reads, recurrence, duplicate detection, batch
planning, mutations, and the EventKit adapter are separate responsibilities.
Within Reminders, selection, query, scheduling, storage, batch planning,
mutations, dispatch, public EventKit conversion, and private hierarchy access
are separate responsibilities.

The refactor toward this layout follows these dependency rules:

- Clap definitions stay under `cli/`. Calendar and Reminders dispatch accept
  those parsed inputs at their command boundary, while provider, adapter, and
  presentation modules remain independent of Clap parsing behavior.
- Travel's canonical flight and itinerary models must remain provider-neutral.
  The FlightAware adapter converts provider data into those models; travel code
  does not reach into FlightAware response types.
- Travel service orchestration is separate from both the provider client and
  the loopback server. The server delegates collection construction to the
  travel service instead of reading Calendar or calling FlightAware directly.
  Server routing and security stay independent of both Calendar and provider
  behavior.
- EventKit and ReminderKit details remain in adapter modules. Selection,
  filtering, recurrence decisions, scheduling, and mutation planning remain
  testable without accessing a real macOS store.
- Private ReminderKit hierarchy access stays isolated in
  `reminders/eventkit/hierarchy.rs` and must not be expanded during the
  refactor.
- Models and output are downstream support modules. They must not dispatch
  commands or initiate Calendar, Reminders, network, or filesystem operations.
- Cross-domain visibility is `pub(crate)` only when a sibling domain needs it;
  implementation details remain private to their module tree.

## Compatibility contracts

The module refactor must preserve the following observable behavior:

- The complete Clap command tree, flags, conflicts, defaults, long help, and
  generated Bash, Elvish, Fish, PowerShell, and Zsh completions.
- Human-readable output and versioned JSON shapes, field names, omission rules,
  warnings, ordering, and exit behavior.
- Calendar and Reminders selection by exact EventKit id, source, and title;
  dry-run and confirmation safeguards; recurrence scope semantics; and batch
  behavior.
- Configuration keys, default values, file permissions, API-key redaction, and
  the behavior of `config path`, `init`, `show`, `validate`, `edit`, `get`,
  `set`, and `unset`.
- Canonical flight-note parsing and formatting, airport metadata, itinerary
  ordering and deduplication, travel schema version 1, warning kinds,
  malformed-record handling, and Calendar-authoritative booked timing versus
  provider-updated timing.
- FlightAware query normalization; deterministic flight/codeshare, route, and
  time matching; normalized status and position behavior; cache paths,
  versions, TTLs, stale fallback, and atomic writes; cross-process monthly quota
  locking and accounting; adaptive backoff and date-window handling; redirect
  refusal and response-size limits; API-key secrecy; and Calendar-only fallback
  when provider data is unavailable.
- The loopback server's capability token, range validation, routing, response
  headers, content types, CSP, and embedded assets. Each launch uses a fresh
  capability token and cookie exchange. The server remains bound only to a
  loopback interface; validates Host, Origin, and fetch-site metadata; accepts
  only GET and HEAD; sends no-store headers; limits inclusive ranges to 366
  days; and reads Calendar without performing writes.

## Refactor rules

- Move one cohesive responsibility at a time and keep compatibility re-exports
  narrow and temporary.
- Do not add dependencies, commands, flags, configuration keys, providers, UI
  behavior, or user-visible features as part of the refactor.
- Keep bundled files under `assets/web/` byte-for-byte stable unless a separate
  issue explicitly changes the travel interface.
- Add or preserve characterization tests before moving behavior. Validate each
  phase independently and audit it before committing.
- Keep implementation files at or below 1,000 lines. A larger file requires an
  explicit, documented justification in the pull request.

## Verification

Run the standard non-mutating checks after each relevant phase and the complete
set before opening the pull request:

```sh
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
node --check assets/web/app.js
node --check tests/browser/travel-ui.spec.mjs
npm run test:browser
git diff --check
```

Run the ignored but read-only EventKit smoke test separately; it reads
authorization and default-calendar state and needs no mutation opt-in:

```sh
cargo test --test eventkit_manual permission_and_default_calendar_are_parseable \
  -- --ignored --nocapture
```

Mutation tests are additional opt-in evidence only. They must use the
repository's explicit environment acknowledgement and exact dedicated test
calendar or reminder-list safeguards, and must never target normal user data.
