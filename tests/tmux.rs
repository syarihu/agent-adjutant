//! Integration tests for tmux terminal backend and `adj tmux` CLI.

mod common;

use common::*;

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
        out.contains("tmux has-session -t '=adjutant-work-test' "),
        "{out}"
    );
    assert!(
        out.contains("new-window -d -t '=adjutant-work-test:'"),
        "{out}"
    );
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
    // The server is already up, so a lookup needs a session to look in...
    let made = tmux.tmux_cmd(&["new-session", "-d", "-s", &tmux.session, "sleep", "60"]);
    assert!(made.status.success(), "{made:?}");
    no_pane_is_found(&Fixture::new(&tmux_config(&tmux.socket, &tmux.session)));
    // ...and a socket nothing listens on is the case of no server at all.
    let nobody = format!("adj-test-nobody-{}", std::process::id());
    no_pane_is_found(&Fixture::new(&tmux_config(&nobody, &tmux.session)));
}

fn no_pane_is_found(fixture: &Fixture) {
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

/// A hub's window is named after the hub, which starts with the session's own name. A spawn
/// after it must still land in the session as a new window, not be aimed at that window.
#[test]
fn tmux_spawn_after_a_window_named_like_the_session() {
    let Some(tmux) = IsolatedTmux::new("spawn-prefix") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let fixture = Fixture::new(&tmux_config(&tmux.socket, &tmux.session));

    let hub_title = format!("{}-hub", tmux.session);
    fixture.ok(&["tmux", "spawn", "--title", &hub_title, "sleep", "60"]);
    fixture.ok(&["tmux", "spawn", "--title", "after-the-hub", "sleep", "60"]);

    let out = tmux.tmux_cmd(&[
        "list-windows",
        "-t",
        &format!("={}:", tmux.session),
        "-F",
        "#{window_name}",
    ]);
    let names = String::from_utf8_lossy(&out.stdout);
    assert!(names.lines().any(|n| n == hub_title), "{names}");
    assert!(names.lines().any(|n| n == "after-the-hub"), "{names}");
}

// ── waking an agent whose screen is read first ───────────────────────

const WAKE_LINE: &str = "wake me up now please";

fn pane_fixture(name: &str) -> String {
    format!(
        "{}/src/fixtures/panes/{name}.txt",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Open a window whose only process is `sh` running `script`, and say which pane it is:
/// its id, and the pid `adj tmux wake` is given.
fn spawn_fake_pane(fixture: &Fixture, title: &str, script: &str) -> (String, String) {
    fixture.ok(&["tmux", "spawn", "--title", title, "sh", "-c", script]);
    let panes = fixture.json(&["tmux", "pane", "--json"]);
    let pane = panes
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["window_name"].as_str() == Some(title))
        .unwrap_or_else(|| panic!("no pane for {title}: {panes}"));
    (
        pane["pane_id"].as_str().unwrap().to_string(),
        pane["pane_pid"].as_u64().unwrap().to_string(),
    )
}

fn screen_of(tmux: &IsolatedTmux, pane_id: &str) -> String {
    let out = tmux.tmux_cmd(&["capture-pane", "-p", "-J", "-t", pane_id]);
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Drawn until the last line of its text is on screen, whatever the machine is doing.
fn wait_for_screen(tmux: &IsolatedTmux, pane_id: &str, needle: &str) -> String {
    for _ in 0..50 {
        let screen = screen_of(tmux, pane_id);
        if screen.contains(needle) {
            return screen;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("{needle:?} never appeared:\n{}", screen_of(tmux, pane_id));
}

#[test]
fn a_question_on_the_agents_screen_is_not_answered_by_a_wake() {
    let Some(tmux) = IsolatedTmux::new("wake-asking") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let fixture = Fixture::new(&tmux_config(&tmux.socket, &tmux.session));
    let script = format!(
        "sed '/@@adjutant:pane@@/,$d' {}; sleep 60",
        shell_quoted(&pane_fixture("claude-question"))
    );
    let (pane_id, pid) = spawn_fake_pane(&fixture, "asking", &script);
    wait_for_screen(&tmux, &pane_id, "Esc to cancel");

    let out = fixture.cmd(&[
        "tmux", "wake", "--pid", &pid, "--agent", "claude", "--line", WAKE_LINE,
    ]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("question or a menu"), "{err}");
    assert!(err.contains("not typed"), "{err}");
    assert!(
        !screen_of(&tmux, &pane_id).contains(WAKE_LINE),
        "the line was typed anyway"
    );

    // Without an agent named, the pane is typed into without looking, as it always was.
    let generic = fixture.ok(&["tmux", "wake", "--pid", &pid, "--line", WAKE_LINE]);
    assert!(generic.contains("woke the session"), "{generic}");
}

#[test]
fn an_agent_at_an_empty_prompt_is_woken() {
    let Some(tmux) = IsolatedTmux::new("wake-idle") else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let fixture = Fixture::new(&tmux_config(&tmux.socket, &tmux.session));
    // As wide as the screens were captured: at 80 columns the fixture's rules wrap, and the
    // rows the script moves the cursor back over are not the lines it printed.
    let made = tmux.tmux_cmd(&[
        "new-session",
        "-d",
        "-s",
        &tmux.session,
        "-n",
        "main",
        "-x",
        "100",
        "-y",
        "30",
    ]);
    assert!(made.status.success(), "{made:?}");
    // The last lines of the fixture above its input line, an input line the terminal echoes
    // into, and the rest of the fixture under it; then the cursor goes back up to the input
    // line and waits for it to be sent.
    let script = format!(
        r#"f={}
n=$(grep -n '❯' "$f" | tail -1 | cut -d: -f1)
sed -n "$((n - 8)),$((n - 1))p" "$f"
printf '❯ \n'
rest=$(sed -n "$((n + 1)),\$p" "$f" | sed '/@@adjutant:pane@@/,$d')
printf '%s\n' "$rest"
printf '\033[%dA\033[3G' $(($(printf '%s\n' "$rest" | wc -l) + 1))
read -r line
echo "sent: $line"
sleep 60"#,
        shell_quoted(&pane_fixture("claude-idle-after-turn"))
    );
    let (pane_id, pid) = spawn_fake_pane(&fixture, "idle", &script);
    wait_for_screen(&tmux, &pane_id, "manual mode on");

    let out = fixture.ok(&[
        "tmux", "wake", "--pid", &pid, "--agent", "claude", "--line", WAKE_LINE,
    ]);
    assert!(out.contains("woke the session"), "{out}");
    wait_for_screen(&tmux, &pane_id, &format!("sent: {WAKE_LINE}"));
}
