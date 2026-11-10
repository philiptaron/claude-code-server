//! Background builds of versions that are not in the catalog.
//!
//! A lookup never waits for one: the server's probe has a time limit, so the newest build
//! answers in the meantime. The builder is an external command run as `BUILDER VERSION`; on
//! success it leaves an entry for the version in the versions directory, which the catalog
//! picks up on the next lookup.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use std::{io, mem, ptr};

use log::{info, warn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Running,
    Failed(Instant),
}

pub struct Builder {
    command: PathBuf,
    retry_after: Duration,
    builds: Arc<Mutex<HashMap<String, State>>>,
}

impl Builder {
    pub fn new(command: PathBuf, retry_after: Duration) -> Builder {
        Builder { command, retry_after, builds: Arc::default() }
    }

    /// Start a build of `version` unless one is running, or one failed less than
    /// `retry_after` ago. Returns whether a build was started.
    pub fn request(&self, version: &str) -> bool {
        {
            let mut builds = self.builds.lock().unwrap();
            match builds.get(version) {
                Some(State::Running) => return false,
                Some(State::Failed(at)) if at.elapsed() < self.retry_after => return false,
                _ => {}
            }
            builds.insert(version.to_owned(), State::Running);
        }
        let command = self.command.clone();
        let builds = Arc::clone(&self.builds);
        let version = version.to_owned();
        info!("building {version} with {}", command.display());
        thread::spawn(move || {
            let mut build = Command::new(&command);
            build
                .arg(&version)
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
            // SAFETY: sigemptyset and pthread_sigmask are async-signal-safe, and the closure
            // touches nothing but its own stack.
            unsafe { build.pre_exec(unblock_signals) };
            let status = build.status();
            let mut builds = builds.lock().unwrap();
            match status {
                Ok(status) if status.success() => {
                    info!("built {version}");
                    builds.remove(&version);
                }
                Ok(status) => {
                    warn!("building {version} failed: {status}");
                    builds.insert(version, State::Failed(Instant::now()));
                }
                Err(error) => {
                    warn!("could not run {}: {error}", command.display());
                    builds.insert(version, State::Failed(Instant::now()));
                }
            }
        });
        true
    }

    #[cfg(test)]
    fn state(&self, version: &str) -> Option<State> {
        self.builds.lock().unwrap().get(version).copied()
    }
}

/// Give the builder an empty signal mask. The daemon blocks the termination signals in every
/// thread (see `block_signals` in main.rs) and a child inherits the mask, so without this
/// the builder and what it runs could not be stopped with SIGTERM.
fn unblock_signals() -> io::Result<()> {
    // SAFETY: the set is initialized by sigemptyset before pthread_sigmask reads it.
    unsafe {
        let mut set: libc::sigset_t = mem::zeroed();
        libc::sigemptyset(&mut set);
        match libc::pthread_sigmask(libc::SIG_SETMASK, &set, ptr::null_mut()) {
            0 => Ok(()),
            errno => Err(io::Error::from_raw_os_error(errno)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_until_settled(builder: &Builder, version: &str) -> Option<State> {
        for _ in 0..500 {
            match builder.state(version) {
                Some(State::Running) => thread::sleep(Duration::from_millis(10)),
                state => return state,
            }
        }
        panic!("build of {version} never finished");
    }

    #[test]
    fn success_clears_state() {
        let builder = Builder::new("true".into(), Duration::from_secs(600));
        assert!(builder.request("1.2.3"));
        assert_eq!(wait_until_settled(&builder, "1.2.3"), None);
        assert!(builder.request("1.2.3"));
    }

    #[test]
    fn failure_backs_off() {
        let builder = Builder::new("false".into(), Duration::from_secs(600));
        assert!(builder.request("1.2.3"));
        assert!(matches!(wait_until_settled(&builder, "1.2.3"), Some(State::Failed(_))));
        assert!(!builder.request("1.2.3"));
        assert!(builder.request("1.2.4"));

        let impatient = Builder::new("/nonexistent/builder".into(), Duration::ZERO);
        assert!(impatient.request("1.2.3"));
        assert!(matches!(wait_until_settled(&impatient, "1.2.3"), Some(State::Failed(_))));
        assert!(impatient.request("1.2.3"));
    }

    #[test]
    fn running_build_is_not_repeated() {
        use std::os::unix::fs::PermissionsExt;
        let script = std::env::temp_dir().join(format!("ccd-cli-fs-slow-{}", std::process::id()));
        std::fs::write(&script, "#!/bin/sh\nsleep 2\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let builder = Builder::new(script.clone(), Duration::ZERO);
        assert!(builder.request("1.2.3"));
        assert!(!builder.request("1.2.3"));
        assert_eq!(wait_until_settled(&builder, "1.2.3"), None);
        std::fs::remove_file(script).unwrap();
    }

    #[test]
    fn builder_runs_with_no_signals_blocked() {
        // Block SIGTERM here, as the daemon does; the builder's thread inherits the mask.
        // SAFETY: the set is initialized by sigemptyset before use.
        unsafe {
            let mut set: libc::sigset_t = mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            assert_eq!(libc::pthread_sigmask(libc::SIG_BLOCK, &set, ptr::null_mut()), 0);
        }
        // Run as `sh SCRIPT`, the script standing where the version does, so nothing
        // executes a file that was just written.
        let script = std::env::temp_dir().join(format!("ccd-cli-fs-mask-{}", std::process::id()));
        std::fs::write(&script, "grep -q '^SigBlk:[[:space:]]*0*$' /proc/self/status\n").unwrap();
        let builder = Builder::new("sh".into(), Duration::from_secs(600));
        let script_name = script.to_str().unwrap();
        assert!(builder.request(script_name));
        assert_eq!(wait_until_settled(&builder, script_name), None);
        std::fs::remove_file(script).unwrap();
    }
}
