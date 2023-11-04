use git2::{RebaseOptions, Repository};
use uuid::Uuid;

use crate::commands::util;

pub fn diff(repository: Repository) {
    let revwalk = util::get_all_commits_from_main(&repository);

    if revwalk.size_hint().0 == 0 {
        println!("Nothing to diff.");
        return;
    }

    // We need to look for the current uuid in the list of commits in the patch
    // We'll need to detect that there is one in the stack, and that all commits have the same one
    let mut detected_uuid: Option<String> = None;

    for id in revwalk {
        match id {
            Ok(oid) => {
                let commit = repository.find_commit(oid).expect("Failed to find commit");

                // uuid found, we need to check whether we already have one stored and if it's the same
                // if not, we'll need to rewrite?
                if let Some(uuid) = util::get_metadata_id_from_commit(&commit) {
                    if detected_uuid.is_none() {
                        detected_uuid = Some(uuid);
                    } else {
                        // TODO
                        let detected = detected_uuid.take().unwrap();

                        if detected != uuid {
                            println!(
                                "Commit {} contains a different uuid than the predecessors. It is {} and it should be {}.",
                                commit.id().to_string(),
                                uuid,
                                detected
                            );
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to get commit: {}", e);
            }
        }
    }

    // Make sure we have a canonical uuid to use
    let stack_uuid = if detected_uuid.is_none() {
        Uuid::new_v4().to_string()
    } else if Uuid::parse_str(detected_uuid.take().unwrap().as_str()).is_err() {
        Uuid::new_v4().to_string()
    } else {
        detected_uuid.unwrap()
    };

    println!("Canonical uuid: {stack_uuid}");

    // We need to walk through all the commits here but in the context of a rebase

    // Get the commit ids for main
    let main_oid = repository
        .revparse_single("main")
        .or_else(|_| repository.revparse_single("master"))
        .expect("Could not find main branch")
        .id();

    let main_commit = repository.find_annotated_commit(main_oid)
        .expect("Could not get annotated commit");

    let mut rebase_options = RebaseOptions::default();
    let mut rebase = repository.rebase(
        None,
        Some(&main_commit),
        None,
        Some(&mut rebase_options))
        .expect("Could not start the rebase");

    let signature = repository.signature().expect("Failed to get signature");

    // Iterate through the rebase operations
    while let Some(Ok(operation)) = rebase.next() {
        match operation.kind() {
            Some(git2::RebaseOperationType::Pick) => {
                let current_commit = repository.find_commit(operation.id()).expect("Could not obtain current commit");
                let current_commit_uuid = util::get_metadata_id_from_commit(&current_commit);

                // Alternatives here:
                //  - if there is no uuid, we need to write one
                //  - if there is an uuid but not the same, we need to overwrite
                //  - if there's our same uuid, it's fine

                let old_message = current_commit.message().unwrap_or("");
                let mut new_message: Option<String> = Some(old_message.to_string());

                if current_commit_uuid.is_none() {
                    new_message = Some(format!("{old_message}\n\ngg-id: {}", stack_uuid));
                } else if current_commit_uuid.unwrap() != stack_uuid {
                    let mut lines: Vec<String> = vec![];
                    for line in old_message.lines() {
                        if line.starts_with("gg-id:") {
                            lines.push(format!("gg-id: {stack_uuid}"));
                        } else {
                            lines.push(line.to_string());
                        }
                    }
                    new_message = Some(lines.join("\n"));
                    println!("Rebuilt message!")
                } // else skip

                // TODO skip the commit if no changes needed for the commit message
                // let commit = repo.find_commit(operation.id().expect("Failed to get commit id")).expect("Failed to find commit");
                // let commit_tree = commit.tree().expect("Failed to get commit tree");
                // let index = repo.index().expect("Failed to get index");
                // let index_tree = index.write_tree().expect("Failed to write tree from index");
                //
                // if commit_tree.id() != index_tree {
                //     // If the commit tree is different from the current index tree, apply the commit
                //     rebase.commit(None, &repo.signature().expect("Failed to get signature"), None).expect("Failed to apply commit");
                //     println!("Applied commit: {:?}", operation.id());
                // } else {
                //     // If the commit tree is identical to the current index tree, skip the commit
                //     println!("Skipped commit: {:?}", operation.id());
                // }

                let new_commit_oid = rebase.commit(None, &signature, new_message.as_deref())
                    .expect("Failed to apply commit");

                let new_current_commit = repository.find_commit(new_commit_oid).expect("Could not find new commit");

                println!("Applied commit: {:?} (gg-id: {})", new_current_commit.id(), stack_uuid);

                let commit_message = new_current_commit.message().unwrap_or("<empty>");
                println!("Commit message:\n{commit_message}");

                // TODO should we push now?
                //  create a new tmp branch, cherry pick this commit, push that branch to the repo?
                //  also hold all temp branches created to delete after this is all done
            }
            _ => {
                // Skip other types of operations
                println!("Skipped commit: {:?}", operation.id());
            }
        }
    }

    // Finalize the rebase
    rebase.finish(None).expect("Failed to finish rebase");

    // printcoln!("[b|u]Commit\t\tStatus\t\tSummary[:]");
    // for id in revwalk {
    //     match id {
    //         Ok(id) => {
    //             let commit = repository.find_commit(id).expect("Failed to find commit");
    //
    //             let mut short_sha = commit.id().to_string();
    //             short_sha.truncate(8);
    //             printcol!("[b|yellow]{}[:]\t", short_sha);
    //             print!("🤷‍\t\t");
    //             println!("Should be pushing to `origin/gg/pr/{}`", commit.id().to_string());
    //         }
    //         Err(e) => eprintln!("Failed to get commit: {}", e),
    //     }
    // }
}

// fn get_or_generate_metadata_for_commit(commit: Commit) -> String {
//     let id = Uuid::new_v4();
//     return id.to_string();
// }
