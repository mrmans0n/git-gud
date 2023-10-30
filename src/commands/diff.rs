use fancy::{printcol, printcoln};
use git2::Repository;

use crate::commands::util;

pub fn diff(repository: Repository) {
    // TODO
    // Next, you can use the following Rust code to push a commit to the specified branch:
    //     rust
    // Copy code
    // extern crate git2;
    //
    // use git2::{Repository, PushOptions, RemoteCallbacks, Cred};
    //
    // fn main() {
    //     // Open the current repository
    //     let repo = Repository::open(".").unwrap();
    //
    //     // Get the remote named "origin"
    //     let mut remote = repo.find_remote("origin").unwrap();
    //
    //     // Set up the push options and callbacks
    //     let mut callbacks = RemoteCallbacks::new();
    //     callbacks.credentials(|_url, _username_from_url, _allowed_types| {
    //         // Provide credentials here, for example:
    //         Cred::ssh_key_from_agent("git")
    //     });
    //     let mut push_options = PushOptions::new();
    //     push_options.remote_callbacks(callbacks);
    //
    //     // Push to the specified branch
    //     remote.push(
    //         &["refs/heads/gg/pr/something:refs/heads/gg/pr/something"],
    //         Some(&mut push_options),
    //     ).unwrap();
    // }
    // In this code:
    //
    //     We first open the current repository using Repository::open.
    //     We then find the remote named "origin" using repo.find_remote.
    //     We create a new PushOptions object and a RemoteCallbacks object to handle authentication and other remote interactions. In this example, we use Cred::ssh_key_from_agent to authenticate using an SSH key, but you may need to adjust this to match your authentication setup.
    //     Finally, we push to the specified branch using remote.push, providing the source and destination refspecs as "refs/heads/gg/pr/something:refs/heads/gg/pr/something" to specify that we are pushing to the gg/pr/something branch on the remote named origin.
    //     Make sure to handle any potential errors that may occur during this process, such as network issues or authentication failures, by checking the result of the unwrap calls or using proper error handling techniques in Rust.


    let revwalk = util::get_all_commits_from_main(&repository);

    printcoln!("[b|u]Commit[:b]\t\t[b|u]Summary[:]");
    println!();
    for id in revwalk {
        match id {
            Ok(id) => {
                let commit = repository.find_commit(id).expect("Failed to find commit");

                let mut short_sha = commit.id().to_string();
                short_sha.truncate(7);
                printcol!(
                    "[b|yellow]{}[:]\t\t",
                    short_sha,
                );
                println!("Should be pushing to `origin/gg/pr/{}`", commit.id().to_string());
            }
            Err(e) => eprintln!("Failed to get commit: {}", e),
        }
    }
}