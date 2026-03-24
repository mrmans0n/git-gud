# Deprecate `auto_add_gg_ids` Implementation Plan

**Goal:** Make `gg` always behave as if `auto_add_gg_ids` were true while keeping backward compatibility for existing configs.
**Architecture:** Keep config deserialization compatible, but remove behavioral branching and setup prompts around the option. Update docs and structured outputs to reflect that the setting is deprecated and effectively always-on.
**Tech Stack:** Rust, serde config model, CLI/setup flow, MCP JSON surface, docs.

---

### Task 1: Deprecate runtime semantics

**Files:**
- Modify: `crates/gg-core/src/config.rs`
- Modify: `crates/gg-core/src/commands/sync.rs`
- Modify: any other command paths that still branch on `auto_add_gg_ids`
- Test: relevant config/runtime tests

**Steps:**
1. Keep `auto_add_gg_ids` deserializable for old configs.
2. Ensure runtime behavior always treats GG-ID auto-add as enabled.
3. Remove stale comments/branches that imply `false` changes behavior.
4. Add/adjust tests to show old configs with `false` no longer disable normalization.

### Task 2: Remove setup prompt

**Files:**
- Modify: `crates/gg-core/src/commands/setup.rs`
- Test: setup/integration coverage if needed

**Steps:**
1. Remove the interactive question about automatically adding GG-IDs.
2. Stop writing the setting from setup.
3. Keep generated config behavior sane and backwards compatible.

### Task 3: Stabilize structured outputs

**Files:**
- Modify: `crates/gg-mcp/src/tools.rs`
- Modify: any config JSON/output structs surfacing the field
- Test: MCP/config JSON tests

**Steps:**
1. Decide on compatibility behavior: keep the field but always emit `true`.
2. Update tests accordingly.

### Task 4: Docs and skill updates

**Files:**
- Modify: `README.md`
- Modify: `docs/src/configuration.md`
- Modify: `docs/src/commands/setup.md`
- Modify: `skills/gg/SKILL.md`
- Modify: `skills/gg/reference.md`

**Steps:**
1. Mark `auto_add_gg_ids` as deprecated.
2. Explain that GG metadata normalization is now mandatory.
3. Clarify that informative messages remain even though the setting is no longer user-controlled.

### Task 5: Verification and PR

**Steps:**
1. Run `cargo fmt --all`.
2. Run `cargo clippy --all-targets --all-features -- -D warnings`.
3. Run `cargo test --all-features`.
4. Open a clean PR with summary, motivation, behavior changes, and compatibility notes.
