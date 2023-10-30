use git2::{Repository, Revwalk};

pub(crate) fn get_all_commits_from_main(repository: &Repository) -> Revwalk {
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
    revwalk
}