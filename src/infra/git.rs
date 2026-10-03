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
    let mut cmd = Command::new("git");
    cmd.args(args);
    for name in REPOSITORY_LOCATION_ENV {
        cmd.env_remove(name);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output().map_err(|e| format!("cannot run git: {e}"))
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
/// Read-only by construction: `GIT_OPTIONAL_LOCKS=0` keeps `status` from refreshing the index
/// behind the back of a worker that is running git in the same worktree. Output is drained on
/// threads, because a listing longer than a pipe holds would otherwise stall the child until
/// the deadline.
///
/// It reads the index and refs of a worktree that is not ours, so beyond the variables that
/// choose the repository it also drops the ones that would swap in another index, object store
/// or ref namespace from whatever environment the server was started in.
pub(crate) fn git_until(
    args: &[&str],
    cwd: &Path,
    deadline: std::time::Instant,
) -> Result<GitOut, String> {
    use std::io::Read;
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    for name in REPOSITORY_LOCATION_ENV.into_iter().chain([
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ]) {
        cmd.env_remove(name);
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot run git: {e}"))?;
    let mut pipe = child.stdout.take().ok_or("cannot read git's output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // Not joined: a grandchild that inherited the pipe keeps it open past the
                // kill, and waiting for its end would hold the deadline hostage. The thread
                // ends with the pipe.
                drop(reader);
                return Err("git did not answer in time".to_string());
            }
            Err(e) => return Err(format!("cannot wait for git: {e}")),
        }
    };
    let bytes = reader.join().unwrap_or_default();
    Ok(GitOut {
        code: status.code(),
        stdout: String::from_utf8_lossy(&bytes).to_string(),
    })
}
