use git2::{RebaseOptions, Repository};
use uuid::Uuid;

use crate::commands::util;

pub fn diff(repository: Repository) {
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
                // TODO mmmh this seems to destroy the repo OOPS
                // let uuid = util::write_metadata_id_to_head(&repository);

                let current_commit = repository.find_commit(operation.id()).expect("Could not obtain current commit");
                let mut uuid = util::get_metadata_id_from_commit(&current_commit);

                let mut new_message: Option<String> = None;

                if uuid == None {
                    // TODO force all commits in the stack to share the same
                    let uuid_string = Uuid::new_v4().to_string();
                    uuid = Some(uuid_string.clone());
                    let current_message_raw = current_commit.message().unwrap_or("");
                    let formatted_message = format!("{current_message_raw}\n\ngg-id: {}", uuid_string);
                    new_message = Some(formatted_message);
                } else {
                    // TODO we should skip here
                }

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

                rebase.commit(None, &signature, new_message.as_deref())
                    .expect("Failed to apply commit");

                let new_current_commit = repository.head().unwrap().peel_to_commit().unwrap();

                println!("Applied commit: {:?} (gg-id: {})", new_current_commit.id(), uuid.unwrap_or("<unset>".to_string()));

                let commit_message = new_current_commit.message().unwrap_or("<empty>");
                println!("Commit message:\n{commit_message}");
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
