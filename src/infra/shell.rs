use std::process::Command;

pub fn run_shell(command: &str) -> Result<String, String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| format!("cannot run the command: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("command failed: {command}")
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Answers whether an executable file is on `PATH`. `adj review-engine` asks it about `codex` too.
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| in_path(&path, program))
}

/// Whether `program` is an executable file in this `PATH`. Split out from the environment so
/// a test can answer for a directory it built rather than for the developer's own machine.
/// A directory of that name, or a file nobody may run, is not a notifier.
pub(super) fn in_path(path: &std::ffi::OsStr, program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(program))
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests;
