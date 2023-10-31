use git2::{RebaseOptions, Repository};
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
                let uuid = util::write_metadata_id_to_head(&repository);

                // let commit_oid = rebase.commit(None, &signature, None)
                //     .expect("Failed to apply commit");

                println!("Applied commit: {:?} (gg-id: {uuid})", operation.id());

                let commit = repository.head().unwrap().peel_to_commit().unwrap();
                let commit_message = commit.message().unwrap_or("<empty>");
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
