//! Integration tests for tmux terminal backend and `adj tmux` CLI.

mod common;

use common::*;

struct IsolatedTmux {
    socket: String,
    session: String,
}

impl IsolatedTmux {
    fn new(name: &str) -> Option<Self> {
        let out = Command::new("tmux").arg("-V").output().ok()?;
        if !out.status.success() {
            return None;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket = format!("adj-test-{name}-{nanos}");
        let session = "adjutant-test".to_string();
        Some(IsolatedTmux { socket, session })
    }

    fn tmux_cmd(&self, args: &[&str]) -> std::process::Output {
        Command::new("tmux")
            .arg("-L")
            .arg(&self.socket)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for IsolatedTmux {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .arg("-L")
            .arg(&self.socket)
            .arg("kill-server")
            .output();
    }
}

fn tmux_config(socket: &str, session: &str) -> String {
    serde_json::json!({
        "notification": "true",
        "terminal": {
            "preset": "tmux",
            "session": session,
            "socket": socket,
        },
        "repos": {
            "acme/widget": {
                "taskSource": "github",
                "issueRepo": "acme/widget",
                "issueKeys": { "acme/widget": "WID" },
                "ide": "code"
            }
        }
    })
    .to_string()
}

fn forge_worker_record(worktree: &Path, pid: u32) -> PathBuf {
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": pid, "title": "WID-957", "psStarted": ps_started(pid)})
            .to_string(),
    )
    .unwrap();
    record
}

#[test]
fn tmux_work_dry_run_generates_tmux_spawn_command() {
    let config = serde_json::json!({
        "notification": "true",
        "terminal": {
            "preset": "tmux",
            "session": "adjutant-work-test",
        },
        "repos": {
            "acme/widget": {
                "taskSource": "github",
                "issueRepo": "acme/widget",
                "issueKeys": { "acme/widget": "WID" },
                "ide": "code"
            }
        }
    })
    .to_string();
    let fixture = Fixture::new(&config);
    let out = fixture.ok(&[
        "work",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--title",
        "WID-100",
        "--dry-run",
    ]);
    assert!(
        out.contains("tmux has-session -t adjutant-work-test"),
        "{out}"
    );
    assert!(out.contains("new-window -d -t adjutant-work-test"), "{out}");
    assert!(out.contains("-n WID-100"), "{out}");
}

#[test]
fn tmux_cli_spawn_pane_wake_focus_and_close_lifecycle() {
    let Some(tmux) = IsolatedTmux::new("lifecycle") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let fixture = Fixture::new(&tmux_config(&tmux.socket, &tmux.session));

    // 1. Spawning a new window
    let spawn_out = fixture.ok(&["tmux", "spawn", "--title", "test-worker", "sleep", "60"]);
    assert!(spawn_out.contains("test-worker"), "{spawn_out}");

    // 2. Listing panes
    let panes_val = fixture.json(&["tmux", "pane", "--json"]);
    let panes = panes_val.as_array().expect("expected array of panes");
    assert!(!panes.is_empty(), "expected at least one pane");

    let worker_pane = panes
        .iter()
        .find(|p| p["window_name"].as_str() == Some("test-worker"))
        .expect("could not find test-worker pane");

    let pane_pid = worker_pane["pane_pid"]
        .as_u64()
        .expect("expected numeric pid") as u32;
    let pane_tty = worker_pane["pane_tty"]
        .as_str()
        .expect("expected tty string");

    // 3. Finding pane by PID
    let by_pid = fixture.json(&["tmux", "pane", "--pid", &pane_pid.to_string(), "--json"]);
    assert_eq!(by_pid["pane_pid"].as_u64(), Some(pane_pid as u64));
    assert_eq!(by_pid["window_name"].as_str(), Some("test-worker"));

    // 4. Finding pane by TTY
    let by_tty = fixture.json(&["tmux", "pane", "--tty", pane_tty, "--json"]);
    assert_eq!(by_tty["pane_tty"].as_str(), Some(pane_tty));
    assert_eq!(by_tty["pane_pid"].as_u64(), Some(pane_pid as u64));

    // 5. Wake dry run
    let wake_dry = fixture.ok(&[
        "tmux",
        "wake",
        "--pid",
        &pane_pid.to_string(),
        "--line",
        "echo wakeup",
        "--dry-run",
    ]);
    assert!(wake_dry.contains("send-keys -l"), "{wake_dry}");
    assert!(wake_dry.contains("echo wakeup"), "{wake_dry}");

    // 6. Wake real execution
    let wake_real = fixture.ok(&[
        "tmux",
        "wake",
        "--pid",
        &pane_pid.to_string(),
        "--line",
        "echo wakeup",
    ]);
    assert!(wake_real.contains("woke the session"), "{wake_real}");

    // 7. Focus dry run
    let focus_dry = fixture.ok(&["tmux", "focus", "--pid", &pane_pid.to_string(), "--dry-run"]);
    assert!(focus_dry.contains("select-window"), "{focus_dry}");
    assert!(focus_dry.contains("select-pane"), "{focus_dry}");

    // 8. Focus real execution
    let focus_real = fixture.ok(&["tmux", "focus", "--pid", &pane_pid.to_string()]);
    assert!(focus_real.contains("focused the tab"), "{focus_real}");

    // 9. Close real execution
    let close_out = fixture.ok(&["tmux", "close", "--pid", &pane_pid.to_string()]);
    assert!(close_out.contains("closed the tab"), "{close_out}");

    // Verify window is now gone
    let after_panes = fixture.json(&["tmux", "pane", "--json"]);
    let still_there = after_panes
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["pane_pid"].as_u64() == Some(pane_pid as u64));
    assert!(!still_there, "pane was not killed by close");
}

#[test]
fn tmux_close_via_adj_close_clears_worker_and_kills_window() {
    let Some(tmux) = IsolatedTmux::new("adj-close") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let fixture = Fixture::new(&tmux_config(&tmux.socket, &tmux.session));

    // Spawn a worker process inside tmux
    fixture.ok(&["tmux", "spawn", "--title", "worker-wid957", "sleep", "60"]);

    let panes_val = fixture.json(&["tmux", "pane", "--json"]);
    let worker_pane = panes_val
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["window_name"].as_str() == Some("worker-wid957"))
        .expect("could not find worker-wid957 pane");
    let pane_pid = worker_pane["pane_pid"].as_u64().unwrap() as u32;

    // Forge worker record in fixture repository
    let record = forge_worker_record(&fixture.repo, pane_pid);
    assert!(record.exists());
    assert!(
        !ps_started(pane_pid).is_empty(),
        "spawned process is not running"
    );

    // Close via `adj close --worktree <repo>`
    let out = fixture.ok(&["close", "--worktree", fixture.repo.to_str().unwrap()]);
    assert!(out.contains("closed the tab"), "{out}");
    assert!(!record.exists(), "worker record was not cleared");

    // Confirm process is dead
    assert!(
        ps_started(pane_pid).is_empty(),
        "process was not reaped/killed after close"
    );
}

#[test]
fn tmux_env_var_override_for_socket_and_session() {
    let Some(tmux) = IsolatedTmux::new("env-override") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    // Config only specifies preset "tmux" with NO socket or session configured
    let config = serde_json::json!({
        "notification": "true",
        "terminal": {
            "preset": "tmux"
        },
        "repos": {
            "acme/widget": {
                "taskSource": "github",
                "issueRepo": "acme/widget",
                "issueKeys": { "acme/widget": "WID" },
                "ide": "code"
            }
        }
    })
    .to_string();
    let fixture = Fixture::new(&config);

    // Run spawn with ADJUTANT_TMUX_SOCKET and ADJUTANT_TMUX_SESSION explicitly set
    let mut cmd = fixture.command(["tmux", "spawn", "--title", "env-worker", "sleep", "60"]);
    cmd.env("ADJUTANT_TMUX_SOCKET", &tmux.socket);
    cmd.env("ADJUTANT_TMUX_SESSION", "custom-session");
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "spawn failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Verify window in custom-session via tmux CLI on that socket
    let check = tmux.tmux_cmd(&[
        "list-windows",
        "-t",
        "custom-session",
        "-F",
        "#{window_name}",
    ]);
    let list_out = String::from_utf8_lossy(&check.stdout);
    assert!(list_out.contains("env-worker"), "{list_out}");
}

#[test]
fn tmux_error_handling_for_non_existent_pane() {
    let Some(tmux) = IsolatedTmux::new("errors") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let fixture = Fixture::new(&tmux_config(&tmux.socket, &tmux.session));

    // Pane lookup for non-existent PID fails
    let out = fixture.cmd(&["tmux", "pane", "--pid", "999999"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no tmux pane found"), "{stderr}");

    // Wake for non-existent PID fails
    let wake_out = fixture.cmd(&["tmux", "wake", "--pid", "999999"]);
    assert!(!wake_out.status.success());
    let wake_err = String::from_utf8_lossy(&wake_out.stderr);
    assert!(wake_err.contains("no tmux pane found"), "{wake_err}");

    // Close for non-existent PID fails
    let close_out = fixture.cmd(&["tmux", "close", "--pid", "999999"]);
    assert!(!close_out.status.success());
    let close_err = String::from_utf8_lossy(&close_out.stderr);
    assert!(close_err.contains("no tmux pane found"), "{close_err}");
}
