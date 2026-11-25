//! Sending `SIGINT` to another process (the follow view stops a detached run that holds
//! its session). Compiled on every target: off Unix the call reports `Unsupported`.

use std::io;

/// Sends `SIGINT` to process `pid` (SA-12 follow view: Ctrl-C stops the detached
/// run that holds the session). Pid 0 is refused: `kill(0, _)` would signal this
/// process group.
#[cfg(unix)]
pub fn send_sigint(pid: u32) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no process id"))?;
    // SAFETY: `kill` only sends a signal to the given pid; it reads no memory of ours.
    if unsafe { libc::kill(pid, libc::SIGINT) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Sends `SIGINT` to process `pid`. Not supported on this platform.
#[cfg(not(unix))]
pub fn send_sigint(_pid: u32) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::send_sigint;

    #[test]
    fn pid_zero_is_refused() {
        // Pid 0 would signal this process group; it must never reach kill().
        assert!(send_sigint(0).is_err());
    }
}
