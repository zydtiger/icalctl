---
name: issue-delivery
description: Pick up an existing GitHub or Gitea issue and deliver it through an isolated worktree, iterative implementation steps, validation, clean-context audits, focused commits, pull-request drafting, approval-gated merge, verification, and cleanup. Use when the user explicitly asks to implement, pick up, complete, or ship an existing forge issue with disciplined review and Git lifecycle management.
---

# Issue Delivery

Deliver an existing issue through explicit planning, implementation, review, publication, merge, and cleanup gates.

## Invariants

- Read and obey repository instructions; they override this generic workflow.
- Require an existing issue and an explicit pickup request. Issue creation alone is not implementation authority.
- Preserve unrelated changes and avoid destructive Git operations.
- Validate, independently audit, and commit each implementation step before the next.
- Never merge or clean up before approval and verification.

## Workflow

### 1. Orient

Resolve the repository, complete issue body and discussion, instructions, linked work, remotes, current state, and base branch. Confirm the issue exists, is open, and is not already implemented, including any changed scope or blockers recorded in comments.

Select the adapter from the remote:

- GitHub: prefer available GitHub integration capabilities; use authenticated `gh` for local-branch gaps.
- Gitea: use authenticated `tea`.

Verify account and target and consult current command help. Never cross-use `gh` and `tea`.

### 2. Propose and approve

Classify the work under repository policy. Use a direct base-branch fix only when explicitly permitted and truly isolated; otherwise use a dedicated worktree.

Propose the branch, base, worktree path, ordered implementation steps, per-step validation, compatibility and migration concerns, the end-to-end final validation suite, and the final pull-request plan. Obtain explicit approval before any branch, worktree, commit, or forge write. Re-plan if scope materially changes.

### 3. Create the worktree

Verify the main checkout is safe, fetch and confirm the approved base, then create the approved branch in a worktree outside the main checkout. Perform all issue edits and commits there. Never reset the main checkout or silently reuse a dirty branch.

### 4. Iterate each implementation step

For every approved step:

1. Implement only that step.
2. Inspect the diff for accidental or unrelated changes.
3. Run formatting, targeted tests, and proportionate regressions.
4. Invoke `$clean-context-code-audit` with original requirements, step scope, raw diff, and test evidence.
5. Verify every finding. Fix valid findings and rerun affected checks.
6. Re-audit when corrections materially alter behavior, architecture, security, or coverage.
7. Stage only intended files and create one focused commit using repository conventions.
8. Verify the commit and clean step boundary.

Do not defer all audits or commits until the end.

### 5. Publish the pull request

Run the full relevant format, test, lint, build, and security suite. Review the complete issue diff and commit sequence and confirm a clean worktree. Push the branch, read [references/pr-body.md](references/pr-body.md), then create or update the pull request through the selected adapter. Verify base, head, remote commit, title, body, issue linkage, and review state. Use draft status only when requested or required.

### 6. Merge gate

Pull-request creation is not merge authorization. After explicit approval, re-read the pull request, confirm its exact head, required checks, reviews, mergeability, and policy. Use the user's merge strategy or explicit repository policy; ask if materially ambiguous. Merge through the correct adapter.

### 7. Verify and clean up

Before deleting anything, verify the merge into the intended base, merge commit, required checks, expected issue state, inclusion of all feature work, and absence of work needing preservation.

Then:

1. Remove the worktree without `--force`.
2. If removal unregisters it but leaves a directory, inspect leftovers and delete only confirmed disposable ignored artifacts under the user's cleanup authorization.
3. Safely delete the local feature branch.
4. Delete the remote feature branch when appropriate.
5. Fetch and prune.
6. Verify the base checkout, worktree list, and branch state.

Never force-remove a dirty worktree or discard unexplained ignored files.
