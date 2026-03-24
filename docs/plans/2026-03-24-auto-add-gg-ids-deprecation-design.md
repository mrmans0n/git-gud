# Deprecate `auto_add_gg_ids` Design

**Status:** approved-in-chat  
**Date:** 2026-03-24

## Decision

`auto_add_gg_ids` no longer makes product sense as a user-controlled behavior.

`gg` now depends on GG metadata invariants (`GG-ID`, and now `GG-Parent`) to keep stack identity and PR/MR mappings stable. If users disable automatic GG-ID insertion, core workflows become unreliable and the tool stops being able to guarantee its own model.

Therefore, `gg` should always behave as if `auto_add_gg_ids = true`.

## Goals

- Deprecate `auto_add_gg_ids` without breaking existing configs immediately.
- Keep GG-ID insertion automatic in all relevant flows.
- Keep informative messaging when metadata is added/fixed.
- Remove the setup-time question since the answer is no longer meaningful.
- Clarify the behavior in docs and structured outputs.

## Non-goals

- Do not introduce a migration command.
- Do not fail on old configs containing `auto_add_gg_ids: false`.
- Do not remove the field from serialized/config-facing APIs in this change if that would cause avoidable compatibility churn.

## Compatibility strategy

Use a soft deprecation:

1. **Config parsing remains compatible**
   - Existing configs with `auto_add_gg_ids` still deserialize.
2. **Runtime behavior ignores false values**
   - All flows behave as if it were true.
3. **Setup no longer prompts for it**
   - New configs stop presenting the option.
4. **Docs mark it deprecated**
   - Explain that GG metadata normalization is now mandatory.
5. **Structured outputs return `true`** where relevant
   - Avoid leaking a misleading false value to downstream consumers.

## Behavior changes

### Before
- Users could configure `auto_add_gg_ids = false`.
- Some flows prompted or respected opt-out behavior.
- Stack invariants could become inconsistent with the direction of the product.

### After
- GG metadata is always normalized as needed.
- Missing GG-IDs are always added when required.
- Informative messages remain.
- The config option is deprecated and effectively ignored.

## Expected code areas

- `crates/gg-core/src/config.rs`
- `crates/gg-core/src/commands/setup.rs`
- `crates/gg-core/src/commands/sync.rs`
- `crates/gg-mcp/src/tools.rs`
- `docs/src/configuration.md`
- `docs/src/commands/setup.md`
- `README.md`
- `skills/gg/SKILL.md`
- `skills/gg/reference.md`

## Risks

- Minor compatibility surprise for users who explicitly set `false`.
- Downstream tooling may still read the field and expect it to be meaningful.

## Mitigation

- Keep parsing compatibility.
- Document the deprecation clearly.
- Return `true` in structured surfaces so consumers do not infer outdated semantics.
