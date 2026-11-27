use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(unix)]
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
use std::os::{fd::AsRawFd, unix::net::UnixStream};
#[cfg(unix)]
use std::process::ExitStatus;
#[cfg(unix)]
use std::time::Duration;

use tempfile::TempDir;

#[cfg(unix)]
const PROCESS_GUARD_CONTROL_FD_ENV: &str = "GG_TEST_PROCESS_GUARD_FD";

#[cfg(unix)]
pub(crate) struct ProcessGroupGuard {
    child: Option<std::process::Child>,
    process_group: i32,
    shutdown: Option<UnixStream>,
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
            shutdown: None,
        })
    }

    #[cfg(test)]
    fn spawn_cooperative(mut command: Command) -> std::io::Result<Self> {
        let (shutdown, child_shutdown) = UnixStream::pair()?;
        let shutdown_fd = shutdown.as_raw_fd();
        let child_shutdown_fd = child_shutdown.as_raw_fd();
        command.env(PROCESS_GUARD_CONTROL_FD_ENV, child_shutdown_fd.to_string());
        command.process_group(0);
        // The control socket remains close-on-exec in the parent. In the child,
        // make only its endpoint inheritable and close the parent's endpoint.
        // Descendants which may create a new session can monitor this descriptor
        // for EOF instead of requiring unsafe discovery and bare-PID signalling.
        //
        // SAFETY: only async-signal-safe libc operations run between fork and
        // exec, and every captured value is a plain file descriptor.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(child_shutdown_fd, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(shutdown_fd);
                Ok(())
            });
        }
        let child = command.spawn()?;
        drop(child_shutdown);
        let process_group = i32::try_from(child.id()).expect("process group fits pid_t");
        Ok(Self {
            child: Some(child),
            process_group,
            shutdown: Some(shutdown),
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.as_ref().expect("guarded child").id()
    }

    pub(crate) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.child.as_mut().expect("guarded child").stdout.take()
    }

    pub(crate) fn wait_with_output_timeout(
        mut self,
        timeout: Duration,
    ) -> std::io::Result<std::process::Output> {
        use std::io::{self, Read};
        use std::time::Instant;

        fn set_nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
            let descriptor = pipe.as_raw_fd();
            // SAFETY: the descriptor remains owned by its ChildStdout/ChildStderr
            // for the duration of both fcntl calls.
            let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
            if flags == -1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: F_SETFL updates flags without taking ownership of the fd.
            if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn drain_pipe(pipe: &mut Option<impl Read>, output: &mut Vec<u8>) -> io::Result<bool> {
            let Some(reader) = pipe.as_mut() else {
                return Ok(true);
            };
            let mut buffer = [0_u8; 8192];
            let mut drained = 0;
            while drained < 64 * 1024 {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        *pipe = None;
                        return Ok(true);
                    }
                    Ok(count) => {
                        output.extend_from_slice(&buffer[..count]);
                        drained += count;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                    Err(error) => return Err(error),
                }
            }
            Ok(false)
        }

        fn drain_or_record(
            pipe: &mut Option<impl Read>,
            output: &mut Vec<u8>,
            first_error: &mut Option<io::Error>,
        ) {
            if let Err(error) = drain_pipe(pipe, output) {
                if first_error.is_none() {
                    *first_error = Some(error);
                }
                *pipe = None;
            }
        }

        let child = self.child.as_mut().expect("guarded child");
        let leader = child.id();
        drop(child.stdin.take());
        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();
        if let Some(pipe) = stdout_pipe.as_ref() {
            set_nonblocking(pipe)?;
        }
        if let Some(pipe) = stderr_pipe.as_ref() {
            set_nonblocking(pipe)?;
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut pipe_error = None;

        let deadline = Instant::now() + timeout;
        let status = loop {
            drain_or_record(&mut stdout_pipe, &mut stdout, &mut pipe_error);
            drain_or_record(&mut stderr_pipe, &mut stderr, &mut pipe_error);
            match self.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    break Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "process tree led by pid {leader} exceeded execution timeout of {timeout:?}"
                        ),
                    ))
                }
                Err(error) => break Err(error),
            }
        };

        // Drop closes the cooperative control descriptor, terminates the owned
        // process group, and reaps the direct child.
        drop(self);
        let status = status?;

        let drain_deadline = Instant::now() + Duration::from_secs(2);
        while (stdout_pipe.is_some() || stderr_pipe.is_some()) && Instant::now() < drain_deadline {
            drain_or_record(&mut stdout_pipe, &mut stdout, &mut pipe_error);
            drain_or_record(&mut stderr_pipe, &mut stderr, &mut pipe_error);
            if stdout_pipe.is_some() || stderr_pipe.is_some() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if let Some(error) = pipe_error {
            return Err(error);
        }
        if stdout_pipe.is_some() || stderr_pipe.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "process pipes did not close after process-tree termination",
            ));
        }
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
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
        // Closing this end asks cooperating descendants (including children of
        // `script` which entered a new PTY session) to exit. Arbitrary escaped
        // descendants cannot be terminated safely by this generic helper on
        // macOS: it deliberately never discovers or signals descendant PIDs.
        drop(self.shutdown.take());
        let Some(mut child) = self.child.take() else {
            return;
        };
        drop(child.stdin.take());
        drop(child.stdout.take());
        drop(child.stderr.take());
        let group = Pid::from_raw(-self.process_group);
        let _ = kill(group, Signal::SIGTERM);
        std::thread::sleep(Duration::from_millis(200));
        let _ = kill(group, Signal::SIGKILL);
        // A direct child can move itself out of the original group. Child still
        // owns and pins this PID until wait, so this cannot target a reused PID.
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{ProcessGroupGuard, PROCESS_GUARD_CONTROL_FD_ENV};
    use nix::errno::Errno;
    use nix::sys::wait::{waitpid, WaitPidFlag};
    use nix::unistd::Pid;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn wait_for_file(path: &Path, label: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {label}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_heartbeat_quiescent(path: &Path) {
        let length = fs::metadata(path).expect("heartbeat exists").len();
        assert!(length > 0, "process never wrote a heartbeat");
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(75));
            assert_eq!(
                fs::metadata(path)
                    .expect("heartbeat remains readable")
                    .len(),
                length,
                "heartbeat changed after process cleanup"
            );
        }
        fs::write(path, "reused").expect("closed heartbeat must be reusable");
    }

    #[cfg(target_os = "linux")]
    fn shell_quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
    }

    fn pty_python_command(program: &Path, arguments: &[&Path]) -> Command {
        let mut command = Command::new("script");
        #[cfg(target_os = "linux")]
        {
            let invocation = std::iter::once("python3".to_string())
                .chain(std::iter::once(shell_quote(program)))
                .chain(arguments.iter().map(|argument| shell_quote(argument)))
                .collect::<Vec<_>>()
                .join(" ");
            command.args(["-q", "-e", "-c", &invocation, "/dev/null"]);
        }
        #[cfg(not(target_os = "linux"))]
        {
            command
                .args(["-q", "/dev/null", "python3"])
                .arg(program)
                .args(arguments);
        }
        command
    }

    fn escaped_process_info(path: &Path) -> (i32, i32) {
        let contents = fs::read_to_string(path).expect("read escaped process identity");
        let mut fields = contents.split_whitespace();
        let pid = fields.next().unwrap().parse().unwrap();
        let process_group = fields.next().unwrap().parse().unwrap();
        (pid, process_group)
    }

    fn receive_bounded<T: Send + 'static>(
        receiver: &std::sync::mpsc::Receiver<T>,
        fallback_stop: &Path,
        label: &str,
    ) -> T {
        match receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(result) => result,
            Err(error) => {
                fs::write(fallback_stop, "stop").expect("request fallback cleanup");
                let cleaned_up = receiver.recv_timeout(Duration::from_secs(2)).is_ok();
                panic!(
                    "{label} exceeded its outer watchdog: {error}; fallback cleanup completed: {cleaned_up}"
                );
            }
        }
    }

    #[test]
    fn wait_with_output_timeout_closes_cooperating_pty_session() {
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("pty-timeout.py");
        let heartbeat = temp.path().join("heartbeat");
        let process_info = temp.path().join("process-info");
        let ready = temp.path().join("ready");
        let control_closed = temp.path().join("control-closed");
        let fallback_stop = temp.path().join("fallback-stop");
        fs::write(
            &program,
            format!(
                r#"import os, select, signal, sys
heartbeat, process_info, ready, control_closed, fallback_stop = sys.argv[1:]
control = int(os.environ[{PROCESS_GUARD_CONTROL_FD_ENV:?}])
signal.signal(signal.SIGHUP, signal.SIG_IGN)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
with open(process_info, 'w') as output:
    output.write(f'{{os.getpid()}} {{os.getpgrp()}}')
open(ready, 'w').close()
while True:
    with open(heartbeat, 'a') as output:
        output.write('x')
    readable, _, _ = select.select([control], [], [], 0.05)
    if readable and os.read(control, 1) == b'':
        open(control_closed, 'w').close()
        break
    if os.path.exists(fallback_stop):
        break
"#
            ),
        )
        .unwrap();
        let mut command = pty_python_command(
            &program,
            &[
                &heartbeat,
                &process_info,
                &ready,
                &control_closed,
                &fallback_stop,
            ],
        );
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let guard = ProcessGroupGuard::spawn_cooperative(command).unwrap();
        let leader_group = i32::try_from(guard.id()).unwrap();
        wait_for_file(&ready, "PTY child readiness");
        let (_escaped_pid, escaped_group) = escaped_process_info(&process_info);
        assert_ne!(
            escaped_group, leader_group,
            "script must place the PTY child in a nested process group"
        );

        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || {
            let _ = send.send(guard.wait_with_output_timeout(Duration::from_millis(250)));
        });
        let result = receive_bounded(&receive, &fallback_stop, "PTY execution-timeout cleanup");
        waiter.join().unwrap();
        let error = result.expect_err("the guarded process must time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            error.to_string().contains("exceeded execution timeout"),
            "primary execution-timeout error was replaced: {error}"
        );
        wait_for_file(&control_closed, "cooperative control-pipe closure");
        assert!(!fallback_stop.exists(), "outer watchdog performed cleanup");
        assert_heartbeat_quiescent(&heartbeat);
    }

    #[test]
    fn wait_with_output_preserves_normal_pty_completion_and_closes_escape() {
        let temp = tempfile::tempdir().unwrap();
        let program = temp.path().join("pty-normal.py");
        let heartbeat = temp.path().join("heartbeat");
        let process_info = temp.path().join("process-info");
        let ready = temp.path().join("ready");
        let control_closed = temp.path().join("control-closed");
        let fallback_stop = temp.path().join("fallback-stop");
        fs::write(
            &program,
            format!(
                r#"import os, select, signal, sys, time
heartbeat, process_info, ready, control_closed, fallback_stop = sys.argv[1:]
control = int(os.environ[{PROCESS_GUARD_CONTROL_FD_ENV:?}])
pid = os.fork()
if pid == 0:
    os.setsid()
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    for descriptor in (0, 1, 2):
        try:
            os.close(descriptor)
        except OSError:
            pass
    with open(process_info, 'w') as output:
        output.write(f'{{os.getpid()}} {{os.getpgrp()}}')
    open(ready, 'w').close()
    while True:
        with open(heartbeat, 'a') as output:
            output.write('x')
        readable, _, _ = select.select([control], [], [], 0.05)
        if readable and os.read(control, 1) == b'':
            open(control_closed, 'w').close()
            break
        if os.path.exists(fallback_stop):
            break
    os._exit(0)
while not os.path.exists(ready):
    time.sleep(0.01)
print('normal PTY output', flush=True)
os._exit(23)
"#
            ),
        )
        .unwrap();
        let mut command = pty_python_command(
            &program,
            &[
                &heartbeat,
                &process_info,
                &ready,
                &control_closed,
                &fallback_stop,
            ],
        );
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let guard = ProcessGroupGuard::spawn_cooperative(command).unwrap();
        let leader_group = i32::try_from(guard.id()).unwrap();
        wait_for_file(&ready, "normal-completion escape readiness");
        let (_escaped_pid, escaped_group) = escaped_process_info(&process_info);
        assert_ne!(escaped_group, leader_group, "escape must own a new session");

        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || {
            let _ = send.send(guard.wait_with_output_timeout(Duration::from_secs(2)));
        });
        let output = receive_bounded(&receive, &fallback_stop, "normal PTY completion")
            .expect("normal PTY command must complete");
        waiter.join().unwrap();
        assert_eq!(output.status.code(), Some(23), "status: {}", output.status);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .matches("normal PTY output")
                .count(),
            1,
            "stdout was not preserved: {:?}",
            String::from_utf8_lossy(&output.stdout)
        );
        wait_for_file(&control_closed, "normal-completion control-pipe closure");
        assert!(!fallback_stop.exists(), "outer watchdog performed cleanup");
        assert_heartbeat_quiescent(&heartbeat);
    }

    #[test]
    fn wait_with_output_timeout_reaps_owned_group_and_closes_pipes() {
        let temp = tempfile::tempdir().unwrap();
        let heartbeat = temp.path().join("heartbeat");
        let process_info = temp.path().join("process-info");
        let ready = temp.path().join("ready");
        let fallback_stop = temp.path().join("fallback-stop");
        let program = r#"
import os, sys, time
heartbeat, process_info, ready, fallback_stop = sys.argv[1:]
with open(process_info, 'w') as output:
    output.write(f'{os.getpid()} {os.getpgrp()}')
pid = os.fork()
if pid == 0:
    while not os.path.exists(fallback_stop):
        with open(heartbeat, 'a') as output:
            output.write('x')
        time.sleep(0.05)
    os._exit(0)
open(ready, 'w').close()
while not os.path.exists(fallback_stop):
    time.sleep(0.05)
"#;
        let mut command = Command::new("python3");
        command
            .arg("-c")
            .arg(program)
            .arg(&heartbeat)
            .arg(&process_info)
            .arg(&ready)
            .arg(&fallback_stop)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let guard = ProcessGroupGuard::spawn(&mut command).unwrap();
        wait_for_file(&ready, "owned-group readiness");
        wait_for_file(&heartbeat, "owned-group heartbeat");
        let (leader_pid, leader_group) = escaped_process_info(&process_info);
        assert_eq!(
            leader_pid, leader_group,
            "leader must own its process group"
        );
        let leader = Pid::from_raw(leader_pid);

        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || {
            let result = guard.wait_with_output_timeout(Duration::from_millis(250));
            let _ = send.send(result);
        });
        let error = receive_bounded(&receive, &fallback_stop, "owned-group timeout cleanup")
            .expect_err("owned group must time out");
        waiter.join().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(!fallback_stop.exists(), "outer watchdog performed cleanup");
        assert_eq!(
            waitpid(leader, Some(WaitPidFlag::WNOHANG)),
            Err(Errno::ECHILD),
            "group leader must be reaped before timeout returns"
        );
        assert_heartbeat_quiescent(&heartbeat);
    }

    #[test]
    fn wait_with_output_closes_piped_stdin_before_polling() {
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                "import sys; sys.stdin.buffer.read(); print('stdin EOF'); sys.stdout.write('x' * 262144); sys.exit(17)",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let guard = ProcessGroupGuard::spawn(&mut command).unwrap();

        let output = guard
            .wait_with_output_timeout(Duration::from_secs(1))
            .expect("piped-stdin child must receive EOF and finish");
        assert_eq!(output.status.code(), Some(17));
        assert!(output.stdout.starts_with(b"stdin EOF\n"));
        assert_eq!(output.stdout.len(), "stdin EOF\n".len() + 262_144);
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn noncooperative_spawn_allows_command_reuse() {
        let mut command = Command::new("python3");
        command
            .args(["-c", "print('reusable command')"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for _ in 0..2 {
            let guard = ProcessGroupGuard::spawn(&mut command).unwrap();
            let output = guard
                .wait_with_output_timeout(Duration::from_secs(1))
                .expect("reused command must not retain stale pre-exec state");
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "reusable command\n"
            );
            assert!(output.stderr.is_empty());
        }
    }

    #[test]
    fn execution_timeout_with_noncooperating_writer_leaves_no_pipe_readers() {
        let temp = tempfile::tempdir().unwrap();
        let heartbeat = temp.path().join("heartbeat");
        let ready = temp.path().join("ready");
        let fallback_stop = temp.path().join("fallback-stop");
        let stopped = temp.path().join("stopped");
        let program = r#"
import os, signal, sys, time
heartbeat, ready, fallback_stop, stopped = sys.argv[1:]
pid = os.fork()
if pid == 0:
    os.setsid()
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    open(ready, 'w').close()
    while not os.path.exists(fallback_stop):
        with open(heartbeat, 'a') as output:
            output.write('x')
        time.sleep(0.05)
    open(stopped, 'w').close()
    os._exit(0)
while not os.path.exists(ready):
    time.sleep(0.01)
print('leader running', flush=True)
while not os.path.exists(fallback_stop):
    time.sleep(0.05)
"#;
        let mut command = Command::new("python3");
        command
            .arg("-c")
            .arg(program)
            .arg(&heartbeat)
            .arg(&ready)
            .arg(&fallback_stop)
            .arg(&stopped)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let guard = ProcessGroupGuard::spawn(&mut command).unwrap();
        wait_for_file(&ready, "non-cooperating escape readiness");

        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let started = Instant::now();
        let waiter = std::thread::spawn(move || {
            let _ = send.send(guard.wait_with_output_timeout(Duration::from_secs(1)));
        });
        let result = receive_bounded(
            &receive,
            &fallback_stop,
            "non-cooperating pipe-drain timeout",
        );
        waiter.join().unwrap();
        let error = result.expect_err("guarded leader must hit its execution timeout");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            error.to_string().contains("exceeded execution timeout"),
            "secondary pipe state replaced the execution error: {error}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));

        fs::write(&fallback_stop, "stop").unwrap();
        wait_for_file(&stopped, "fallback escape cleanup");
        assert_heartbeat_quiescent(&heartbeat);
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
