# AI collaboration guide

This repository uses the checked-in `memory-bank/` as its durable project context. Read it before proposing or changing implementation work.

## Required operating loop

1. Read `memory-bank/projectbrief.md`, `memory-bank/activeContext.md`, and the relevant task entry in `memory-bank/tasks/_index.md`.
2. Inspect the affected code and the current Git diff. Do not invent product, architecture, credentials, or deployment facts.
3. Make the smallest coherent change, keeping unrelated user changes intact.
4. Run the narrowest relevant verification. State exactly what was run and what remains unverified.
5. Update `memory-bank/activeContext.md`, `memory-bank/progress.md`, and the task or decision record when the work changes project knowledge.

## Source-of-truth rules

- Source code and deployed configuration are authoritative for current behavior.
- `memory-bank/` is the maintained explanation of that behavior, decisions, risks, and work state; it is not a replacement for verification.
- Never put secrets, access tokens, personal data, production exports, or copied logs in repository documentation.
- Treat generated content, external APIs, destructive operations, and schema changes as high risk: confirm scope and preserve rollback information.

## Definition of done

An implementation is done only when its tests or other appropriate checks pass, its user-facing behavior is verified when applicable, and the durable memory reflects the result. Mark missing access, devices, credentials, or live-provider checks as blockers rather than assuming success.

## Commit message standard

When generating a commit message for this repository:

- Use a concise, context-specific, imperative title. Aim for 50 characters and do not exceed 72 characters.
- Separate the title from the body with a blank line.
- Include a contextual body that explains the purpose and every material addition, change, and removal. Wrap body text at approximately 72 characters.
- Name the affected behavior or scope, explain why it changed, and include relevant verification, migration, or compatibility notes.
- Never generate a title-only message or a vague title such as `Updates` or `Fixes`.

## Keeping this guide useful

Keep instructions concise, repository-specific, and actionable. Add a nested `AGENTS.md` when a subsystem needs rules that differ materially from these.
