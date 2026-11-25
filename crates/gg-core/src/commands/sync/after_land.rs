//! Evidence for publishing a stack after its bottom entries landed.

use git2::{Oid, Repository};
use std::time::{Duration, Instant};

use crate::error::{GgError, Result};
use crate::git;
use crate::provider::{BaseMergeability, PrInfo, PrState, Provider};
use crate::stack::{Stack, StackEntry};

pub(super) struct LandedPrefix {
    pub len: usize,
    pub rewritten: bool,
    pub previous_target: Option<String>,
}

fn contains(repo: &Repository, tip: Oid, commit: Oid) -> Result<bool> {
    Ok(tip == commit || repo.graph_descendant_of(tip, commit)?)
}

/// The fetch can remove entries from the displayed stack, even when their review
/// mappings have already been forgotten. Object identity needs no forge lookup.
/// For a squash, require both the original source and the actual merge result.
pub(super) fn landed_prefix(
    repo: &Repository,
    stack: &Stack,
    provider: &Provider,
    base: Oid,
) -> Result<Option<LandedPrefix>> {
    let mut len = 0;
    let mut rewritten = false;
    let mut first_target = None;
    for entry in &stack.entries {
        if !contains(repo, base, entry.oid)? {
            let Some(number) = entry.mr_number else {
                // An equivalent patch is not authority to discard a local
                // commit. Nor should a lost mapping create a second review.
                if entry.gg_id.is_some() && !contains(repo, entry.oid, base)? {
                    let parent = repo.find_commit(entry.oid)?.parent_id(0)?;
                    let cherry = git::run_git_command(&[
                        "cherry",
                        &base.to_string(),
                        &entry.oid.to_string(),
                        &parent.to_string(),
                    ])?;
                    if cherry.starts_with("- ") {
                        return Err(GgError::Other(format!("Entry {} matches a patch on origin/{} but has no review mapping; reconcile it before syncing.", entry.short_sha, stack.base)));
                    }
                }
                break;
            };
            let info = provider.get_pr_info(number)?;
            if info.state != PrState::Merged {
                first_target = info.base_branch;
                break;
            }
            let merge = info
                .merge_sha
                .as_deref()
                .and_then(|s| Oid::from_str(s).ok());
            if !matches_source(
                &info,
                number,
                stack.entry_branch_name(entry).as_deref(),
                entry.oid,
            ) || info.base_branch.as_deref() != Some(&stack.base)
                || !merge
                    .map(|oid| contains(repo, base, oid))
                    .transpose()?
                    .unwrap_or(false)
            {
                return Err(GgError::Other(format!(
                    "Cannot verify that {} landed on origin/{}; inspect the merged review before syncing.",
                    entry.short_sha, stack.base
                )));
            }
            rewritten = true;
        }
        len += 1;
    }

    let previous_target = if len > 0 {
        stack.entry_branch_name(&stack.entries[len - 1])
    } else {
        // The user's local base (or a previous conditional sync) may already
        // exclude the parent. A GG-Parent trailer alone isn't ancestry proof.
        let Some(id) = stack.landed_parent_gg_id(repo, Some(base))? else {
            return Ok(None);
        };
        if first_target.as_deref() == Some(&stack.base)
            && git::sync_base_oid(repo, &stack.branch_name(), &stack.base)?
                .is_some_and(|previous| previous != base)
        {
            // This transition was already synced. A later unrelated advance
            // of the base follows the configured behind-base policy.
            return Ok(None);
        }
        Some(git::format_entry_branch(&stack.username, &stack.name, id))
    };
    Ok(Some(LandedPrefix {
        len,
        rewritten,
        previous_target,
    }))
}

fn matches_source(info: &PrInfo, number: u64, branch: Option<&str>, oid: Oid) -> bool {
    info.number == number
        && branch.is_some()
        && info.head_branch.as_deref() == branch
        && info.head_sha.as_deref().and_then(|s| Oid::from_str(s).ok()) == Some(oid)
}

/// Retarget only the reviewed source we fetched. An old mergeability result
/// from the former target must never authorize skipping the rebase.
pub(super) fn needs_rebase(
    repo: &Repository,
    stack: &Stack,
    published_entry: &StackEntry,
    previous_target: Option<&str>,
    provider: &Provider,
    mut before_retarget: impl FnMut() -> Result<()>,
) -> Result<bool> {
    let Some(number) = published_entry.mr_number else {
        // There's no existing review whose mergeability we can ask about. A
        // published branch with a lost mapping needs manual reconciliation.
        if stack
            .entry_branch_name(published_entry)
            .and_then(|name| git::get_remote_branch_oid(repo, &name))
            .is_some()
        {
            return Err(GgError::Other(format!(
                "No review mapping for published entry {}; reconcile the stack before syncing.",
                published_entry.short_sha
            )));
        }
        return Ok(true);
    };
    let branch = stack.entry_branch_name(published_entry);
    let remote_oid = branch
        .as_deref()
        .and_then(|name| git::get_remote_branch_oid(repo, name))
        .ok_or_else(|| {
            GgError::Other(format!(
                "No published source for review {number}; reconcile the stack before syncing."
            ))
        })?;
    let locally_amended = remote_oid != published_entry.oid;
    if locally_amended {
        // A normal local amendment leaves gg's local entry branch at its last
        // published SHA. A fetched head that disagrees with both is ambiguous.
        let last_published = branch
            .as_ref()
            .and_then(|name| repo.refname_to_id(&format!("refs/heads/{name}")).ok());
        if last_published != Some(remote_oid) {
            return Err(GgError::Other(format!(
                "Published source does not match {}; reconcile the stack before syncing.",
                published_entry.short_sha
            )));
        }
    }
    let read = || -> Result<PrInfo> {
        let info = provider.get_pr_info(number)?;
        if !matches!(info.state, PrState::Open | PrState::Draft)
            || !matches_source(&info, number, branch.as_deref(), remote_oid)
        {
            return Err(GgError::Other(format!(
                "Review {number} is not open at the expected source {}; sync stopped.",
                published_entry.short_sha
            )));
        }
        Ok(info)
    };
    let mut info = read()?;
    let target = Some(stack.base.as_str());
    if info.base_branch.as_deref() != target {
        if info.base_branch.is_none() || info.base_branch.as_deref() != previous_target {
            return Err(GgError::Other(format!(
                "Review {number} has an unexpected target; inspect it before syncing."
            )));
        }
        before_retarget()?;
        provider.update_pr_base(number, &stack.base)?;
        info = read()?;
    }
    let start = Instant::now();
    loop {
        if info.base_branch.as_deref() != target {
            return Err(GgError::Other(format!(
                "Review {number} does not report {} as its target; retry sync once it updates.",
                stack.base
            )));
        }
        if locally_amended {
            // Mergeability describes the old published version, not this edit.
            return Ok(true);
        }
        match info.base_mergeability {
            BaseMergeability::MergesAsIs => return Ok(false),
            BaseMergeability::NeedsRebase => return Ok(true),
            BaseMergeability::Unknown if start.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_secs(2));
                info = read()?;
            }
            BaseMergeability::Unknown => return Err(GgError::Other(format!("Review {number} mergeability on {} is unknown; retry sync once the forge has checked it.", stack.base))),
        }
    }
}

/// Don't silently rewrite an already published suffix to repair unrelated
/// trailer damage. New, unpublished entries still get their metadata normally.
pub(super) fn check_preserved_metadata(repo: &Repository, stack: &Stack) -> Result<()> {
    let mut would_rewrite = false;
    for (entry, parent) in stack.entries_with_expected_parents(repo)? {
        would_rewrite |= entry.gg_id.is_none() || entry.gg_parent.as_deref() != parent;
        if would_rewrite
            && stack
                .entry_branch_name(entry)
                .is_some_and(|branch| git::get_remote_branch_oid(repo, &branch) == Some(entry.oid))
        {
            return Err(GgError::Other(format!("Repairing GG metadata would rewrite published entry {}; inspect the stack before syncing.", entry.short_sha)));
        }
    }
    Ok(())
}

/// Rebase only the surviving suffix onto the fetched OID. In particular, do
/// not check out or update the local base, which may belong to another worktree.
pub(super) fn rebase_survivors(
    repo: &Repository,
    stack: &Stack,
    provider: &Provider,
    base: Oid,
    skip: usize,
    before_mutation: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if git::current_branch_name(repo).as_deref() != Some(&stack.branch_name()) {
        return Err(GgError::Other(
            "Check out the stack branch before rebasing its remaining entries.".into(),
        ));
    }
    for entry in &stack.entries[skip..] {
        if let Some(number) = entry.mr_number {
            let info = provider.get_pr_info(number)?;
            if info.number != number
                || info.head_branch != stack.entry_branch_name(entry)
                || !matches!(info.state, PrState::Open | PrState::Draft)
            {
                return Err(GgError::ImmutableTargetsDuringSync(format!(
                    "Cannot safely rewrite {} (review {number}).",
                    entry.short_sha
                )));
            }
        }
    }
    let upstream = if skip == 0 {
        repo.find_commit(stack.entries[0].oid)?.parent_id(0)?
    } else {
        stack.entries[skip - 1].oid
    };
    before_mutation()?;
    // A successful replay can put merge commits from origin/<base> in our
    // history. Discovery (including `gg continue` after a conflict) must
    // exclude those commits even if refs/heads/<base> is still stale.
    git::record_sync_base(repo, stack, base)?;
    match git::run_git_command(&[
        "rebase",
        "--no-update-refs",
        "--onto",
        &base.to_string(),
        &upstream.to_string(),
    ]) {
        Err(_) if git::is_rebase_in_progress(repo) => Err(GgError::RebaseConflict),
        result => result.map(|_| ()),
    }
}
