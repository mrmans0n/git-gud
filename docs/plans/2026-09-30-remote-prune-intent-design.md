# Version-bound remote prune intent

## Invariants

- A remote entry branch is deletable only from durable intent created by the successful local `gg drop` that removed it.
- Each intent names the branch, the originating Drop operation, and the exact `origin/<branch>` OID known before the local rewrite. A later remote lookup may verify that OID but never replace it as authority.
- Missing trusted remote-tracking state at Drop creates no intent. This fails closed: a possibly published branch is retained until a later explicit Drop has a trusted version.
- Sync cancels intent for every branch present in the complete local stack before considering deletion. `--until` never narrows that protection.
- Historical Drop journal records are audit/undo data only and never become deletion authority.

## Lifecycle

1. Drop snapshots candidate branch names and their remote-tracking OIDs before rewriting. Its operation plan carries these candidates across conflict/`gg continue`.
2. After Drop completes, candidates become `pending` intents in repository-local config. Undo of that Drop cancels its intents before restoring refs. Recreating an entry also cancels matching intent durably.
3. Sync compares the live remote branch with the intent's expected OID. Absence consumes the intent. A different OID fails with an actionable error and preserves pending intent without deleting.
4. Before issuing an exact force-with-lease delete, sync persists `deleting` and marks the operation as touching the remote. Any unsuccessful attempt remains `deleting`: stderr cannot prove that the server did not apply a request whose response was lost. Confirmed success records `RemoteEffect::BranchDeleted` with the authorized OID, then consumes the intent. Each branch advances independently so partial success is durable.
5. A leftover `deleting` state means the prior outcome could not be persisted conclusively. Sync fails closed and requires manual reconciliation; it does not retry deletion, including if the same OID was republished.

## Lock ordering

Every full repository-config read-modify-write flow acquires the shared operation lock before loading config, and begins any operation record only from that locked snapshot. Continuation and Land reload config after acquiring or reacquiring the lock; Abort holds the same lock for its state mutation. This ordering prevents a waiting writer from restoring deletion authority that an earlier operation consumed. Atomic rename protects an individual config write from tearing, but does not make an unlocked read-modify-write transaction safe.

## Compatibility and durability

The additive config field defaults empty and is repository-local; global config cannot supply deletion intent. Existing configs and operation records continue to deserialize. The bounded operation journal remains unchanged, while pending intent survives journal rotation.

Remote mutation and local persistence cannot be atomic. Persisting `deleting` first prevents an ambiguous successful delete from becoming an automatic repeat deletion. It may conservatively strand an intent after a crash or persistence failure; manual remote inspection and config repair are then required rather than claiming a transactional or power-loss durability guarantee. The config writer uses atomic rename but does not fsync the file and parent directory.
