# icalctl Agent Guide

## Project

`icalctl` is an Apple Silicon macOS CLI written in Rust for reading and managing the local Apple Calendar store through EventKit. It works with calendars already configured in Calendar.app, including iCloud, Google, Exchange, and local calendars. Human-readable output is the default; `--json` is the scripting and agent interface.

Calendar writes are real user-data mutations. Preserve the confirmation and calendar-selection safeguards documented in `skills/icalctl-skill/SKILL.md`, and prefer exact EventKit calendar ids in automated workflows.

## Agent Skill Installation

The bundled agent skill is a complete directory containing its guarded iTerm
fallback helper. Install and pin it globally with `skillctl`:

```sh
skillctl --global add https://github.com/zydtiger/icalctl.git \
  --path skills/icalctl-skill \
  --ref dev
```

After this directory-based installation, use
`skillctl --global update icalctl-skill` to advance it. A legacy installation
that selected only the repository-root `SKILL.md` must be removed and added
again once to adopt the complete directory.

## Project Skill Dependencies

`.agents/skills.lock.yaml` pins the shared workflows that this repository's
issue process requires:

- `$issue-discovery` drafts and creates approved GitHub issues;
- `$issue-delivery` implements existing issues through the required worktree,
  review, publication, and cleanup gates;
- `$clean-context-code-audit` supplies the independent review gate used by
  fixes and implementation phases.

Treat their vendored directories and `.skillctl-managed` markers as read-only.
Make shared changes in the `agent-workflows` warehouse, then apply them here
with `skillctl update`. Run `skillctl check` before relying on these workflows.

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

## GitHub Issue Workflow

Use GitHub Issues as the canonical backlog for planned features, bugs, and other
actionable work. Do not recreate already resolved historical items as GitHub
issues. Document current behavior in `README.md` and
`skills/icalctl-skill/SKILL.md`; use commits and pull requests for implementation
history.

### Drafting and creating issues

When the user mentions a feature request or bug, first search the repository for
an existing issue. If none exists, draft an issue in the conversation for the
user to review; do not create it on GitHub yet. A useful draft contains:

- a concise title and proposed labels;
- the problem, motivation, and relevant current behavior;
- concrete scope and important implementation constraints;
- acceptance criteria and required test coverage;
- explicit out-of-scope items, risks, or compatibility concerns when relevant.

Create the GitHub issue only after the user explicitly approves the draft.
Immediately before creating it, restate the exact repository, title, labels, and
intended write. Preserve the approved detail in the issue body rather than
reducing it to a short summary.

Always query the repository's current labels before proposing or applying them.
The currently available labels are `bug`, `documentation`, `duplicate`,
`enhancement`, `good first issue`, `help wanted`, `invalid`, `question`, and
`wontfix`. Use `bug` for defects and `enhancement` for feature requests. There is
no `feature-request` label at present; do not invent or create a missing label
without separate user approval. Add secondary labels only when they accurately
describe the work.

Prefer the connected GitHub integration for issue reads and writes. If it can
read the repository but a requested write fails for lack of integration
permission, verify the repository and active account, then fall back to the
authenticated `gh` CLI.

### Picking up an issue

When the user explicitly asks to pick up a GitHub issue, inspect the issue and
current repository state, then classify it as either a simple single-step bug
fix or phased work.

For phased work, propose both of the following for approval:

- a branch name: `feat/issue-<number>-<slug>` for a feature,
  `fix/issue-<number>-<slug>` for a bug, or another Conventional Commit-aligned
  prefix when more accurate;
- ordered implementation phases, each small enough to implement, review, test,
  and commit independently, with the validation planned for each phase.

Do not create the branch, worktree, commits, or other GitHub writes until the
user explicitly approves the branch name and phases.

After approval, create a dedicated Git worktree in the project parent directory,
not inside the main checkout. Use a predictable path such as
`../icalctl-issue-<number>-<slug>` and create the approved branch from the
up-to-date `origin/dev`. Keep all issue implementation, tests, and commits in
that worktree. Preserve unrelated changes in the main checkout.

### Simple bug-fix exception

A bug may use a direct-on-`dev` workflow only when it is a small, isolated,
single-step correction with one focused commit and no meaningful design choice,
migration, dependency change, multi-component implementation, or staged rollout.
If the fix expands beyond that boundary, stop and switch to the phased worktree
workflow after obtaining user approval.

Before editing, propose the direct-fix scope and validation plan and obtain the
user's explicit approval. Do not use this exception when the main checkout is
not on `dev`, contains unrelated or overlapping uncommitted changes, is behind
`origin/dev`, or otherwise cannot isolate the bug safely; use a dedicated
worktree instead.

For an approved simple bug fix:

1. Work directly in the main project folder on `dev`; do not create a feature
   branch, worktree, or pull request.
2. Implement only the isolated fix and its focused regression test or other
   proportionate validation.
3. Run formatting, targeted tests, and relevant regression checks, then launch
   a clean-context subagent to review the diff and evidence. Fix every valid
   finding and repeat review when material corrections were required.
4. Commit only the bug-fix files with a focused `fix:` Conventional Commit
   subject after all findings and checks are clear.
5. Push the commit to `origin/dev`, verify the exact commit is present there,
   and add a GitHub issue comment that explicitly names and links the resolving
   commit and summarizes the validation performed.
6. Manually close the issue as `completed` only after the push and comment
   succeed. No PR is required for this exception.

Never close the issue merely because the local fix works; the resolving commit
must be available on GitHub and identified in the closing conversation.

### Implementing phases

Implement the approved phases in order. For each phase:

1. Implement only that phase's scoped changes in the issue worktree.
2. Run formatting, targeted tests, and proportionate regression checks.
3. Launch a clean-context subagent with no inherited conversation turns to
   review the issue requirements, current phase, diff, and test evidence for
   correctness, regressions, missing tests, documentation gaps, and scope creep.
4. Fix every valid finding and rerun the relevant checks. If fixes materially
   change the phase, request another clean-context review; repeat until no
   actionable problems remain.
5. Commit the cleared phase using the repository's Conventional Commit rules.
   Keep each phase commit focused and do not include unrelated files.

Do not collapse all phases into one final commit, and do not commit a phase
before its review findings and validation failures are resolved.

### Pull request and merge gate

After every approved phase is implemented and committed, run the full relevant
test and lint suite in the worktree and review the complete issue diff. Push the
feature branch and create a GitHub pull request targeting `dev`. The PR body
must include:

- a clear summary of what changed;
- important implementation details explaining how it works and why that design
  was chosen;
- phase/commit structure, validation performed, compatibility or private-API
  risks, and any remaining limitations;
- `Closes #<number>` so GitHub closes the issue only when the PR is merged.

Creating the PR does not authorize merging it. Never merge the PR, close the
issue manually, delete the worktree, or delete the branch until the user has
reviewed the PR and explicitly approved the merge. After approval, merge using
the user's chosen strategy, verify that the issue closed, and then clean up the
worktree and branch as appropriate.

After GitHub reports a successful merge, perform post-merge cleanup explicitly
and in this order:

1. Verify that the PR is merged into `dev`, required checks passed, and the
   linked issue closed.
2. Confirm the issue worktree has no uncommitted changes, untracked files that
   need preserving, or commits that were not included in the merged PR.
3. Remove the issue worktree with `git worktree remove <path>` without
   `--force`. Never force-remove a dirty or locked worktree merely to finish
   cleanup.
4. Delete the local feature branch with `git branch -d <branch>`. If a squash or
   rebase merge prevents Git from recognizing it as merged, use `-D` only after
   independently verifying the PR merge and confirming that the branch contains
   no unique work that must be preserved.
5. Delete the remote feature branch with `git push origin --delete <branch>` if
   GitHub did not already delete it automatically.
6. Run `git fetch --prune` and verify `git worktree list` and branch status no
   longer contain stale issue entries.

Do not delete the base `dev` branch or remove any unrelated worktree or branch.

## Releases

Do not bump the version on every commit or without explicit user approval. Use
Semantic Versioning: patch for compatible fixes, minor for new functionality or
breaking changes while pre-1.0, and major for breaking changes after 1.0. When a
coherent, tested, documented set of changes is substantial enough to release,
suggest the version and rationale to the user and wait for approval before
changing `Cargo.toml` or creating a release tag. Create a matching Git tag for
every approved release using `v<version>`, for example `v0.1.0`, and always
push that tag explicitly to `origin` after creating it.
