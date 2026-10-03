use super::*;

/// The controlling terminal of a process, as `ttys004`. `None` means it has none.
pub fn tty_of(pid: u32) -> Option<String> {
    let out = Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    tty_in(&String::from_utf8_lossy(&out.stdout))
}

/// The tty in what `ps -o tty=` printed, if it named one.
///
/// Split out because the two systems this runs on spell "none" differently — `??` on macOS,
/// `?` on Linux — and a machine only ever demonstrates its own. Taken for a tty name, either
/// one sends every caller looking through a terminal's tabs for `/dev/?`, which no session
/// can be sitting on: `close` would then report having closed nothing, and say so as a
/// failure the cleanup stops on.
pub(super) fn tty_in(printed: &str) -> Option<String> {
    match printed.trim() {
        "" | "?" | "??" => None,
        tty => Some(tty.to_string()),
    }
}

/// This process's terminal, found by walking up the process tree.
///
/// A tool invoked by an agent often has no controlling terminal of its own, but one of its
/// ancestors is the session sitting in the tab — that is the one being named.
pub fn own_tty() -> Option<String> {
    let mut pid = std::process::id();
    for _ in 0..16 {
        if let Some(tty) = tty_of(pid)
            && std::path::Path::new(&format!("/dev/{tty}")).exists()
        {
            return Some(tty);
        }
        pid = parent_of(pid)?;
        if pid <= 1 {
            return None;
        }
    }
    None
}

pub(super) fn parent_of(pid: u32) -> Option<u32> {
    let out = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}
