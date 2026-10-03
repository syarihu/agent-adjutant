use std::path::Path;
use std::process::Command;

/// The variables with which git is told which repository to use, ahead of the directory it is
/// started in.
///
/// Every question asked here is about the repository a directory belongs to, so git has to
/// find it from that directory. Git exports `GIT_DIR` to the hooks it runs, and a person can
/// have one set; left in place, any of these would answer for another checkout — its hub
/// name, its inbox, its worker count — with nothing reporting the swap. `GIT_INDEX_FILE`,
/// `GIT_OBJECT_DIRECTORY` and the like stay: they do not choose the repository.
/// The agent `adj hub` and `adj work` start is handed an environment without them too.
pub(crate) const REPOSITORY_LOCATION_ENV: [&str; 3] =
    ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR"];

/// Git, answering for the repository `cwd` (or the current directory) is in.
pub(crate) fn git(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output, String> {
    run(args, cwd, None, false)
}

/// One pipe of git's output, read to its end on a thread of its own.
fn drain<R: std::io::Read + Send + 'static>(
    mut pipe: R,
) -> std::io::Result<std::thread::JoinHandle<Vec<u8>>> {
    std::thread::Builder::new().spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

/// The one place git is started, so that what is cleared from its environment is decided once.
///
/// `looking` is for a run that only looks at a worktree that is not ours: beyond the variables
/// that choose the repository it also drops the ones that would swap in another index, object
/// store or ref namespace from whatever environment the server was started in, and sets
/// `GIT_OPTIONAL_LOCKS=0` so `status` does not refresh the index behind the back of a worker
/// that is running git in the same worktree.
///
/// Output is drained on threads, because a listing longer than a pipe holds would otherwise
/// stall the child. With a `deadline` the child is killed when it passes.
fn run(
    args: &[&str],
    cwd: Option<&Path>,
    deadline: Option<std::time::Instant>,
    looking: bool,
) -> Result<std::process::Output, String> {
    use std::process::Stdio;
    let mut cmd = Command::new("git");
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for name in REPOSITORY_LOCATION_ENV {
        cmd.env_remove(name);
    }
    if looking {
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
        for name in [
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
        ] {
            cmd.env_remove(name);
        }
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot run git: {e}"))?;
    // Dropping a `Child` neither kills nor waits, so a return from here with git still running
    // would leave it behind unreaped; every way out before the wait below ends it first.
    let abandon = |child: &mut std::process::Child, msg: String| {
        let _ = child.kill();
        let _ = child.wait();
        Err(msg)
    };
    let (Some(out_pipe), Some(err_pipe)) = (child.stdout.take(), child.stderr.take()) else {
        return abandon(&mut child, "cannot read git's output".to_string());
    };
    let out_reader = match drain(out_pipe) {
        Ok(reader) => reader,
        Err(e) => return abandon(&mut child, format!("cannot read git's output: {e}")),
    };
    // A first reader already started is only dropped on failure here: it ends with its pipe
    // once git is dead.
    let err_reader = match drain(err_pipe) {
        Ok(reader) => reader,
        Err(e) => return abandon(&mut child, format!("cannot read git's output: {e}")),
    };
    let status = match deadline {
        None => child
            .wait()
            .map_err(|e| format!("cannot wait for git: {e}"))?,
        Some(deadline) => loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Not joined: a grandchild that inherited the pipe keeps it open past the
                    // kill, and waiting for its end would hold the deadline hostage. The
                    // threads end with the pipes.
                    drop(out_reader);
                    drop(err_reader);
                    return Err("git did not answer in time".to_string());
                }
                Err(e) => return Err(format!("cannot wait for git: {e}")),
            }
        },
    };
    Ok(std::process::Output {
        status,
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
    })
}

pub(crate) struct GitOut {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: String,
}

impl GitOut {
    pub(crate) fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// Git in `cwd`, killed when `deadline` passes.
///
/// Read-only by construction: it is run as a look at a worktree that is not ours, with what
/// `run` clears and sets for that.
pub(crate) fn git_until(
    args: &[&str],
    cwd: &Path,
    deadline: std::time::Instant,
) -> Result<GitOut, String> {
    let out = run(args, Some(cwd), Some(deadline), true)?;
    Ok(GitOut {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
    })
}
