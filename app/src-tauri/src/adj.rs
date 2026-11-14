//! Finding the `adj` binary and the resident server it manages.
//!
//! Everything runs through the login shell: a Finder-launched app has a bare PATH, and the server
//! it may start inherits its environment (tmux, gh), so it has to be the one the person's shell sets up.

use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

const FALLBACKS: [&str; 3] = [
    "~/.cargo/bin/adj",
    "/opt/homebrew/bin/adj",
    "/usr/local/bin/adj",
];
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const START_WAIT: Duration = Duration::from_secs(15);

struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Ran {
    /// What to show a person when the command failed: both streams, stdout first.
    fn text(&self) -> String {
        let mut parts = vec![];
        for s in [&self.stdout, &self.stderr] {
            let s = s.trim();
            if !s.is_empty() {
                parts.push(s.to_owned());
            }
        }
        parts.join("\n")
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn expand_home(p: &str) -> String {
    match (p.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => p.to_owned(),
    }
}

/// What a reader has seen so far, and a signal for when it reached the end of the pipe.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    eof: mpsc::Receiver<()>,
}

/// A reader that keeps what it has seen: a daemon the command left behind may hold the pipe open
/// long after the command itself is done, so its end is waited for only for a moment.
fn drain<R: Read + Send + 'static>(mut from: R) -> Drain {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&buf);
    let (done, eof) = mpsc::channel();
    thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = from.read(&mut chunk) {
            if n == 0 {
                break;
            }
            into.lock().unwrap().extend_from_slice(&chunk[..n]);
        }
        let _ = done.send(());
    });
    Drain { buf, eof }
}

impl Drain {
    /// Everything the command wrote: the reader is given a moment to take the last of it once the
    /// command is gone, and what it has by then is returned if a daemon still holds the pipe.
    fn finish(self) -> String {
        let _ = self.eof.recv_timeout(Duration::from_secs(2));
        String::from_utf8_lossy(&self.buf.lock().unwrap()).into_owned()
    }
}

fn run_login_shell(script: &str) -> Result<Ran, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_owned());
    let mut child = Command::new(&shell)
        .args(["-l", "-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{shell} を起動できません: {e}"))?;
    let out = drain(child.stdout.take().expect("stdout is piped"));
    let err = drain(child.stderr.take().expect("stderr is piped"));

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > COMMAND_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} 秒たっても終わりません: {script}",
                    COMMAND_TIMEOUT.as_secs()
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(Ran {
        ok: status.success(),
        stdout: out.finish(),
        stderr: err.finish(),
    })
}

/// The `adj` to run: the configured path, else the login shell's PATH, else the usual install places.
pub fn find_adj(configured: Option<&str>) -> Result<String, String> {
    if let Some(p) = configured {
        let p = expand_home(p);
        return if Path::new(&p).is_file() {
            Ok(p)
        } else {
            Err(format!("設定の adjPath が見つかりません: {p}"))
        };
    }
    if let Ok(ran) = run_login_shell("command -v adj") {
        // Shell start-up files may print before the answer: it is the last line.
        if let Some(line) = ran
            .stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            && ran.ok
            && line.starts_with('/')
            && Path::new(line).is_file()
        {
            return Ok(line.to_owned());
        }
    }
    FALLBACKS
        .iter()
        .map(|p| expand_home(p))
        .find(|p| Path::new(p).is_file())
        .ok_or_else(|| {
            "adj が見つかりません。インストールするか、設定ファイルの adjPath にパスを書いてください。"
                .to_owned()
        })
}

struct ServerStatus {
    running: bool,
    url: Option<String>,
}

/// `adj server status --json` prints JSON and exits 1 when the server is down, so stdout is read
/// whatever the exit code; a shell's start-up noise before the first `{` is skipped.
fn status(adj: &str) -> Result<ServerStatus, String> {
    let ran = run_login_shell(&format!("{} server status --json", quote(adj)))?;
    let start = ran
        .stdout
        .find('{')
        .ok_or_else(|| format!("adj server status の出力を読めません\n{}", ran.text()))?;
    let value = serde_json::Deserializer::from_str(&ran.stdout[start..])
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok)
        .ok_or_else(|| format!("adj server status の出力を読めません\n{}", ran.text()))?;
    Ok(ServerStatus {
        running: value
            .get("running")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        url: value.get("url").and_then(Value::as_str).map(str::to_owned),
    })
}

/// The board's root URL (with its token): the running resident server's, started first if it is not running.
pub fn discover(configured: Option<&str>) -> Result<String, String> {
    let adj = find_adj(configured)?;
    let current = status(&adj)?;
    if current.running
        && let Some(url) = current.url
    {
        return Ok(url);
    }

    let started = run_login_shell(&format!("{} server start --no-open", quote(&adj)))?;
    if !started.ok {
        return Err(format!(
            "adj server start が失敗しました\n{}",
            started.text()
        ));
    }
    let since = Instant::now();
    loop {
        let now = status(&adj)?;
        if now.running
            && let Some(url) = now.url
        {
            return Ok(url);
        }
        if since.elapsed() > START_WAIT {
            return Err(format!(
                "サーバーを起動しましたが、応答がありません\n{}",
                started.text()
            ));
        }
        thread::sleep(Duration::from_millis(400));
    }
}
