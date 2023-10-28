use fancy::printcoln;
use git2::Repository;

pub fn list_commits_off_of_main(repository: Repository) {
    let mut revwalk = repository.revwalk().expect("Could not create revwalk");

    // Get the commit ids for HEAD and main
    let head = repository
        .revparse_single("HEAD")
        .expect("Could not find HEAD");
    let main = repository
        .revparse_single("main")
        .or_else(|_| repository.revparse_single("master"))
        .expect("Could not find main branch");

    // Push the range to the revwalk
    revwalk
        .push_range(&format!("{}..{}", main.id(), head.id()))
        .expect("Could not push range between main..head commits");

    printcoln!("[b|u]Commit[:b]\t\t[b|u]Summary[:]");
    println!();
    for id in revwalk {
        match id {
            Ok(id) => {
                // TODO will need to show in the future whether the patches are pushed already, etc
                let commit = repository.find_commit(id).expect("Failed to find commit");
                let mut short_sha = commit.id().to_string();
                short_sha.truncate(7);
                printcoln!(
                    "[b|yellow]{}[:]\t\t{}",
                    short_sha,
                    commit.summary().unwrap_or("<no summary>")
                );
            }
            Err(e) => eprintln!("Failed to get commit: {}", e),
        }
    }
}
