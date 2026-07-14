---
name: clean-context-code-audit
description: Run an independent, clean-context audit of an implementation diff and its validation evidence using a fresh subagent, then triage findings and repeat after material corrections. Use when a code change, implementation phase, bug fix, or pre-commit gate requires review without inherited conversation, leaked conclusions, or confirmation bias.
---

# Clean-Context Code Audit

Use a fresh reviewer as an evaluation surface. Pass raw task artifacts rather than the implementing agent's conclusions.

## Integrity rules

- Launch a new subagent with no inherited turns when supported.
- Do not reveal expected answers, suspected defects, previous findings, or a preferred verdict.
- Give enough raw evidence to reconstruct the task.
- Keep review read-only: no edits, commits, pushes, or external writes.
- Exclude secrets and irrelevant user data.
- Independently verify every finding.

If a clean subagent cannot be launched, report that the required gate could not be completed; do not silently substitute self-review.

## Workflow

### 1. Build the packet

Provide the absolute repository or worktree path, applicable instructions, original issue or task requirements, current step scope, the exact raw diff under review, exact validation commands and results, relevant logs/screenshots/fixtures, and neutrally stated environment limits. Include base/head identifiers only as supplemental provenance; never substitute them for the raw diff, especially when the worktree has uncommitted changes.

Exclude conversation history, non-required rationale, suspected findings, and previous reviewer conclusions.

### 2. Launch a fresh reviewer

Use a neutral prompt equivalent to:

> Audit this implementation against the supplied requirements. Read applicable repository instructions, inspect the diff and surrounding code, and assess correctness, regressions, security, missing tests, documentation gaps, and scope creep. Report only actionable findings prioritized P0 through P3, with file and narrow line references, evidence, impact, and the missing or failing test where applicable. If none exist, say so explicitly. Do not edit files or create commits.

Allow safe inspection and validation commands.

### 3. Triage

Reproduce each finding and classify it as valid, invalid, already covered, or outside scope. Fix every valid in-scope finding, rerun affected checks, and reject findings only with concrete evidence.

- **P0:** catastrophic or release-blocking.
- **P1:** high-impact correctness, security, or data-loss risk.
- **P2:** ordinary defect or meaningful regression.
- **P3:** low-impact actionable defect or coverage gap.

Ignore style preferences unless they violate repository rules or cause a defect.

### 4. Re-audit

After material behavioral, architectural, security, or coverage corrections, build a new packet and launch another fresh reviewer without prior conclusions. Repeat until no actionable findings remain and validation passes. Mechanical non-behavioral corrections may be checked directly unless policy requires another audit.

## Exit evidence

Report audit scope, supplied and rerun validation, fixed or rejected findings with reasons, whether re-audit occurred, and final actionable-finding status. Never clear the gate with a valid finding or required failing check.
