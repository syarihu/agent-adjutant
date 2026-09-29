//! A child process on a pseudo-terminal of its own, for the board terminal: `tmux attach` has
//! to see a terminal, and the only terminal on this side of the browser is one made here.
//!
//! Unix only, and the one place this crate calls into libc. The child gets the slave side as
//! stdin, stdout and stderr and a session of its own with that terminal as its controlling one;
//! this side keeps the master, which is what carries bytes to and from it.
//!
//! A leaf: standard library and libc.

use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Pty {
    master: File,
    child: Child,
    /// Set once the child has been waited for: its pid may be someone else's after that, so
    /// nothing signals it again.
    reaped: bool,
}

fn window(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn cloexec(fd: &impl AsRawFd) -> io::Result<()> {
    let fd = fd.as_raw_fd();
    // SAFETY: plain fcntl calls on a descriptor this function was just given.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Run `command` on a new pseudo-terminal of `cols` x `rows`. The caller's environment is
/// inherited except for what `command` itself was told to set or remove.
pub fn spawn(mut command: Command, cols: u16, rows: u16) -> io::Result<Pty> {
    let (mut master, mut slave) = (-1, -1);
    let mut size = window(cols, rows);
    // SAFETY: both out-parameters are valid for writes, the name and termios are optional, and
    // the window size is only read.
    let made = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if made != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openpty returned these two descriptors and nothing else owns them.
    let (master, slave) = unsafe { (File::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    // Neither may leak into a child some other thread is starting at the same moment.
    cloexec(&master)?;
    cloexec(&slave)?;
    command
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // SAFETY: only async-signal-safe calls between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // The `Stdio`s die with `command`'s spawn, so this side ends up holding no slave: what is
    // left of it is in the child, and its exit is what makes the master read EOF (or EIO).
    let child = command.spawn()?;
    drop(command);
    Ok(Pty {
        master,
        child,
        reaped: false,
    })
}

impl Pty {
    #[cfg(test)]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Another handle on the master, for a thread that only reads. Reading it ends with EOF or
    /// an error once the child is gone.
    pub fn reader(&self) -> io::Result<File> {
        self.master.try_clone()
    }

    /// What was typed, on its way to the child.
    pub fn writer(&self) -> &File {
        &self.master
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let size = window(cols, rows);
        // SAFETY: TIOCSWINSZ only reads the struct it is given.
        let done = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) };
        if done < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Ask the child to go, the way a closing terminal window would.
    fn hangup(&self) {
        // SAFETY: a signal to a child this value has not yet reaped.
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGHUP);
        }
    }

    #[cfg(test)]
    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// End the child and reap it: hang up, give it `grace` to leave, then kill. Always waits,
    /// so no zombie is left however it went. Safe to call again, and it is what dropping does.
    pub fn shutdown(&mut self, grace: Duration) {
        if self.reaped {
            return;
        }
        self.hangup();
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                self.reaped = true;
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reaped = true;
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::mpsc;

    /// What the child prints, chunk by chunk, so a wait for it can time out instead of hanging
    /// the test run.
    fn output_of(pty: &Pty) -> mpsc::Receiver<String> {
        let mut reader = pty.reader().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0
                    || tx
                        .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                        .is_err()
                {
                    break;
                }
            }
        });
        rx
    }

    fn wait_for(rx: &mpsc::Receiver<String>, seen: &mut String, wanted: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !seen.contains(wanted) {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(chunk) => seen.push_str(&chunk),
                Err(_) => panic!("never saw {wanted:?} in {seen:?}"),
            }
        }
    }

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn the_child_sees_a_terminal_of_the_size_asked_for_and_follows_a_resize() {
        let mut pty = spawn(sh("stty size; read x; stty size"), 100, 30).unwrap();
        let rx = output_of(&pty);
        let mut seen = String::new();
        wait_for(&rx, &mut seen, "30 100");
        pty.resize(120, 40).unwrap();
        pty.writer().write_all(b"\n").unwrap();
        wait_for(&rx, &mut seen, "40 120");
        pty.shutdown(Duration::from_secs(1));
    }

    #[test]
    fn a_hung_up_child_is_reaped() {
        let mut pty = spawn(sh("sleep 60"), 80, 24).unwrap();
        let pid = pty.pid() as libc::pid_t;
        pty.shutdown(Duration::from_secs(5));
        // Reaped: the pid is no longer ours to signal, rather than a zombie that still is.
        // SAFETY: signal 0 only checks whether the process exists.
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        assert!(!alive, "child {pid} is still there");
    }

    #[test]
    fn a_child_that_ignores_the_hangup_is_killed_after_the_grace() {
        let mut pty = spawn(sh("trap '' HUP; while :; do sleep 1; done"), 80, 24).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        pty.shutdown(Duration::from_millis(200));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(pty.try_wait().unwrap().is_some());
    }

    #[test]
    fn the_reader_ends_when_the_child_does() {
        let mut pty = spawn(sh("echo bye"), 80, 24).unwrap();
        let rx = output_of(&pty);
        let mut seen = String::new();
        wait_for(&rx, &mut seen, "bye");
        // The channel closes when the reader thread sees EOF or EIO.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => assert!(Instant::now() < deadline, "the reader never ended"),
            }
        }
        pty.shutdown(Duration::from_secs(1));
    }

    #[test]
    fn dropping_a_pty_reaps_its_child() {
        let pty = spawn(sh("sleep 60"), 80, 24).unwrap();
        let pid = pty.pid() as libc::pid_t;
        drop(pty);
        // SAFETY: signal 0 only checks whether the process exists.
        assert!(
            unsafe { libc::kill(pid, 0) } != 0,
            "child {pid} is still there"
        );
    }
}
