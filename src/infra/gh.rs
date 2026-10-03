use std::io::Read;
use std::time::Instant;

/// What a `gh` run printed, and whether it exited cleanly.
pub(crate) struct GhRun {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Run `gh` (from `dir`, when given) and collect what it printed. `Err` only when it could not
/// be run or did not finish by `deadline`: a non-zero exit is an answer too, and `gh api -i`
/// prints a 304 and exits 1.
///
/// Both pipes are read while the process runs: an answer can be far longer than a pipe holds,
/// and a `gh` blocked on a full pipe would look like a hang and be killed at the deadline.
pub(crate) fn run(dir: Option<&str>, args: &[&str], deadline: Instant) -> Result<GhRun, String> {
    run_with_input(dir, args, None, deadline)
}

/// `run`, with `input` on `gh`'s stdin when given (and nothing, closed, when not). Also `Err`
/// when a `gh` that exited cleanly did not take all of `input`.
///
/// The bytes are written on a thread of their own: a body bigger than a pipe holds, to a `gh`
/// that has stopped reading, must not keep the caller past its deadline.
pub(crate) fn run_with_input(
    dir: Option<&str>,
    args: &[&str],
    input: Option<&[u8]>,
    deadline: Instant,
) -> Result<GhRun, String> {
    let mut command = std::process::Command::new("gh");
    command
        .args(args)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|e| format!("cannot run gh: {e}"))?;
    fn drain(pipe: Option<impl Read + Send + 'static>) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    }
    let written = match (input, child.stdin.take()) {
        (Some(input), Some(mut stdin)) => {
            // Dropped with the thread, so `gh` sees the end of the body. Not joined: on a kill
            // the pipe breaks and the write ends by itself. How it went is read below, but only
            // from a `gh` that said success: otherwise its own stderr is the better answer.
            let input = input.to_vec();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(std::io::Write::write_all(&mut stdin, &input));
            });
            Some(rx)
        }
        _ => None,
    };
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    // The readers are never joined, on the kill paths or after a clean exit: a process `gh`
    // started may still hold a pipe open, and waiting for it would hold the caller past its
    // deadline.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("gh did not answer in time".to_string());
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cannot wait for gh: {e}"));
            }
        }
    };
    // A pipe still held open after a clean exit is waited out to the deadline and then reported
    // as a timeout, though `gh` said success: what it printed cannot be taken as complete.
    let collect = |rx: std::sync::mpsc::Receiver<Vec<u8>>| match rx
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        Ok(bytes) => Ok(bytes),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err("gh did not answer in time".to_string())
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err("cannot read gh: the reader panicked".to_string())
        }
    };
    let out = collect(out)?;
    let err = collect(err)?;
    if let Some(rx) = written.filter(|_| status.success()) {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("cannot hand gh the input: {e}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err("gh did not answer in time".to_string());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("cannot hand gh the input: the writer stopped".to_string());
            }
        }
    }
    Ok(GhRun {
        ok: status.success(),
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).trim().to_string(),
    })
}
