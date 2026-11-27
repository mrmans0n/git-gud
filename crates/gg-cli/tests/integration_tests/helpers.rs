use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(target_os = "macos")]
use nix::libc;
#[cfg(target_os = "linux")]
use nix::sys::wait::{waitid, Id, WaitPidFlag, WaitStatus};
#[cfg(unix)]
use nix::{
    sys::signal::{kill, Signal},
    unistd::Pid,
};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::process::ExitStatusExt;
#[cfg(unix)]
use std::process::ExitStatus;
#[cfg(unix)]
use std::time::Duration;

use tempfile::TempDir;

#[cfg(unix)]
pub(crate) struct ProcessGroupGuard {
    child: Option<std::process::Child>,
    process_group: i32,
}

#[cfg(unix)]
impl ProcessGroupGuard {
    pub(crate) fn spawn(command: &mut Command) -> std::io::Result<Self> {
        command.process_group(0);
        let child = command.spawn()?;
        let process_group = i32::try_from(child.id()).expect("process group fits pid_t");
        Ok(Self {
            child: Some(child),
            process_group,
        })
    }

    pub(crate) fn poll_exit_status_non_reaping(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let child = self
            .child
            .as_ref()
            .expect("guarded child was already reaped");
        poll_exit_without_reaping(i32::try_from(child.id()).expect("child pid fits pid_t"))
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.poll_exit_status_non_reaping()
    }
}

#[cfg(target_os = "linux")]
fn poll_exit_without_reaping(pid: i32) -> std::io::Result<Option<ExitStatus>> {
    let flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG | WaitPidFlag::WNOWAIT;
    let status = waitid(Id::Pid(Pid::from_raw(pid)), flags)
        .map_err(|error| std::io::Error::from_raw_os_error(error as i32))?;
    Ok(match status {
        WaitStatus::StillAlive => None,
        WaitStatus::Exited(_, code) => Some(ExitStatus::from_raw(code << 8)),
        WaitStatus::Signaled(_, signal, core_dumped) => Some(ExitStatus::from_raw(
            signal as i32 | i32::from(core_dumped) << 7,
        )),
        _ => None,
    })
}

#[cfg(target_os = "macos")]
fn poll_exit_without_reaping(pid: i32) -> std::io::Result<Option<ExitStatus>> {
    let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: `info` points to writable, zeroed storage for `siginfo_t`; waitid
    // initializes it for the requested owned child. WNOWAIT preserves the
    // zombie, so this guard continues to own the leader PID/PGID until Drop.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: waitid returned success, so the zeroed value is initialized even
    // when WNOHANG reports no state change (identified by si_pid == 0).
    let info = unsafe { info.assume_init() };
    if info.si_pid == 0 {
        return Ok(None);
    }
    let raw = match info.si_code {
        libc::CLD_EXITED => info.si_status << 8,
        libc::CLD_KILLED => info.si_status,
        libc::CLD_DUMPED => info.si_status | 0x80,
        code => {
            return Err(std::io::Error::other(format!(
                "waitid returned unexpected child status code {code}"
            )))
        }
    };
    Ok(Some(ExitStatus::from_raw(raw)))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn poll_exit_without_reaping(_pid: i32) -> std::io::Result<Option<ExitStatus>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "non-reaping process polling is supported on Linux and macOS",
    ))
}

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let group = Pid::from_raw(-self.process_group);
        let _ = kill(group, Signal::SIGTERM);
        std::thread::sleep(Duration::from_millis(200));
        let _ = kill(group, Signal::SIGKILL);
        let _ = child.wait();
    }
}

pub(crate) fn isolated_home(repo_path: &std::path::Path) -> PathBuf {
    let home = repo_path.join(".test-home");
    fs::create_dir_all(&home).expect("Failed to create isolated test home");
    home
}

pub(crate) fn create_test_repo() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let repo_path = temp_dir.path().to_path_buf();

    // Initialize git repo with explicit main branch (for CI compatibility)
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git name");

    // Create initial commit on main
    fs::write(repo_path.join("README.md"), "# Test Repo\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create initial commit");

    (temp_dir, repo_path)
}

/// Helper to run gg command in a repo
pub(crate) fn run_gg(repo_path: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let gg_path = env!("CARGO_BIN_EXE_gg");

    let output = Command::new(gg_path)
        .args(args)
        .current_dir(repo_path)
        .env("HOME", isolated_home(repo_path))
        .output()
        .expect("Failed to run gg");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (output.status.success(), stdout, stderr)
}

pub(crate) fn run_gg_with_env(
    repo_path: &std::path::Path,
    args: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
) -> (bool, String, String) {
    let gg_path = env!("CARGO_BIN_EXE_gg");

    let mut cmd = Command::new(gg_path);
    cmd.args(args)
        .current_dir(repo_path)
        .env("HOME", isolated_home(repo_path));
    for (key, value) in envs {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("Failed to run gg");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    (output.status.success(), stdout, stderr)
}

/// Helper to run git command
pub(crate) fn run_git(repo_path: &std::path::Path, args: &[&str]) -> (bool, String) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    (output.status.success(), stdout)
}

pub(crate) fn run_git_full(repo_path: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

pub(crate) fn create_test_repo_with_remote() -> (TempDir, PathBuf, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let base_path = temp_dir.path().to_path_buf();

    // Create a bare repo to act as remote
    let remote_path = base_path.join("remote.git");
    fs::create_dir_all(&remote_path).expect("Failed to create remote dir");

    Command::new("git")
        .args(["init", "--bare", "--initial-branch=main"])
        .current_dir(&remote_path)
        .output()
        .expect("Failed to init bare repo");

    // Create the working repo
    let repo_path = base_path.join("repo");
    fs::create_dir_all(&repo_path).expect("Failed to create repo dir");

    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git name");

    // Add remote
    Command::new("git")
        .args(["remote", "add", "origin", remote_path.to_str().unwrap()])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add remote");

    // Create initial commit on main
    fs::write(repo_path.join("README.md"), "# Test Repo\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create initial commit");

    // Push to remote
    Command::new("git")
        .args(["push", "-u", "origin", "main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to push to remote");

    (temp_dir, repo_path, remote_path)
}

pub(crate) fn create_test_repo_with_worktree_support() -> (TempDir, PathBuf) {
    let parent_dir = TempDir::new().expect("Failed to create parent temp dir");
    let repo_path = parent_dir.path().join("repo");
    fs::create_dir(&repo_path).expect("Failed to create repo dir");

    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git name");

    fs::write(repo_path.join("README.md"), "# Test Repo\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to create initial commit");

    (parent_dir, repo_path)
}

/// Helper to create a git worktree as a sibling of the repo inside the parent temp dir
pub(crate) fn create_worktree(main_repo: &PathBuf, name: &str) -> PathBuf {
    let worktree_path = main_repo.parent().unwrap().join(name);
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            worktree_path.to_str().unwrap(),
            "-b",
            name,
        ])
        .current_dir(main_repo)
        .output()
        .expect("Failed to run git worktree add");
    assert!(
        output.status.success(),
        "Failed to create worktree '{}': {}{}",
        name,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    worktree_path
}
