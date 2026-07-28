# Native clients

## Preconditions

- Load this reference only for MCP or native-client integration.
- Keep CLI semantics canonical and map tools to the corresponding CLI behavior.
- Prefer read-only inspection tools before mutation.

## Procedure

1. Pass a new `--client-operation-id <ID>` on every mutation.
2. For targeted undo, find the exact flag and value pair in
   `gg undo --list --json`, then use that record's opaque `op_...` ID. Never
   infer a record by timestamp or ordering.
3. Use `gg sc --staged-only` for a client-prepared index.
4. Use split Describe/Apply only for a client-owned hunk picker. Structured
   Apply has no force override.
5. Require the same land, drop, force, and admin authority as the CLI
   workflows.

Keep only decision-critical protocol fields here. Use the
[mdBook command reference](../../../docs/src/commands/README.md) and source for
complete schemas.

## Stop conditions

Stop on a missing client operation ID, ambiguous operation record or target,
missing mutation authority, unsupported installed CLI behavior, or a failed
structured Apply.

## Verification

Use the corresponding read-only CLI inspection to verify stack order, `HEAD`,
working-tree state, operation correlation, and any affected remote state.

## Report

Report the native operation, client operation ID, opaque undo record ID when
created, CLI-equivalent behavior, affected state, and remaining authority or
compatibility blockers.
