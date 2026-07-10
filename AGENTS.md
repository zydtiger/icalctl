# icalctl Agent Guide

## Project

`icalctl` is an Apple Silicon macOS CLI written in Rust for reading and managing the local Apple Calendar store through EventKit. It works with calendars already configured in Calendar.app, including iCloud, Google, Exchange, and local calendars. Human-readable output is the default; `--json` is the scripting and agent interface.

Calendar writes are real user-data mutations. Preserve the confirmation and calendar-selection safeguards documented in `SKILL.md`, and prefer exact EventKit calendar ids in automated workflows.

## Agent Skill Installation

The recommended installation for the bundled agent skill is to copy the
project's `SKILL.md` to `~/.agents/skills/icalctl-skill/SKILL.md`:

```sh
mkdir -p ~/.agents/skills/icalctl-skill
cp SKILL.md ~/.agents/skills/icalctl-skill/SKILL.md
```

Run these commands from the project root, and copy the file again after
updating the repository so the installed skill stays current.

## Git Commit Prefixes

Use Conventional Commit-style subjects in the form `prefix: concise imperative summary`:

- `feat:` new user-facing functionality
- `fix:` bug fixes or correctness changes
- `docs:` documentation-only changes
- `test:` test-only additions or corrections
- `refactor:` internal restructuring without a behavior change
- `perf:` performance improvements
- `build:` dependency or build-system changes
- `ci:` continuous-integration changes
- `chore:` repository maintenance that fits none of the above

Keep each commit focused. Use a lowercase prefix, omit a trailing period, and add an optional scope only when it makes the subject clearer, for example `feat(calendar): add stable id selection`.
