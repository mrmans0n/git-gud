use fancy::printcoln;
use git2::Repository;

use crate::commands::util;

pub fn list_commits_off_of_main(repository: Repository) {
    let revwalk = util::get_all_commits_from_main(&repository);

    if revwalk.size_hint().0 == 0 {
        return;
    }

    // TODO we should traverse revwalk backwards to list!

    printcoln!("[b|u]Commit[:b]\t\t[b|u]Summary[:]");
    for id in revwalk {
        match id {
            Ok(id) => {
                // TODO will need to show in the future whether the patches are pushed already, etc
                let commit = repository.find_commit(id).expect("Failed to find commit");
                let mut short_sha = commit.id().to_string();
                short_sha.truncate(8);
                printcoln!(
                    "[b|yellow]{}[:]\t{}",
                    short_sha,
                    commit.summary().unwrap_or("<no summary>")
                );
            }
            Err(e) => eprintln!("Failed to get commit: {}", e),
        }
    }
}
