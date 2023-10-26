use git2::BranchType::Local;
use git2::ObjectType::Commit;
use git2::Repository;

pub fn create_branch_off_of_main(repository: Repository, branch_name: String) {
    // Find the main branch
    let main_branch = repository.find_branch("main", Local)
        .or_else(|_| repository.find_branch("master", Local))
        .expect("Could not find main branch");

    // Create the branch off of main
    let new_branch = repository.branch(&*branch_name, &main_branch.get().peel_to_commit().unwrap(), false)
        .expect("Could not create the new branch");

    // Checkout said branch
    repository.checkout_tree(&new_branch.get().peel(Commit).unwrap(), None).expect("Could not check out new branch");

    // Set the new branch as the current head
    repository.set_head(&new_branch.get().name().unwrap()).expect("Could not set the new branch as the current head");
}

