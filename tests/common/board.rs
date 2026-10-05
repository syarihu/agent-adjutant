//! What the board tests share: a fake tmux, a resident on it, worktrees, records, gates and
//! the state read.

use super::*;

/// `/api/boards`, as a list of objects.
pub fn boards_of(resident: &Resident) -> Vec<serde_json::Value> {
    let (status, body) = resident.get("/api/boards");
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

pub fn board_of(resident: &Resident, slug: &str) -> serde_json::Value {
    boards_of(resident)
        .into_iter()
        .find(|b| b["slug"] == slug)
        .unwrap_or_else(|| panic!("no board {slug}"))
}

/// A `tmux` that writes down what it was asked, answers `list-panes` from a file, and closes
/// a pane by killing the process the test names. Nothing here reaches a real tmux server.
pub struct FakeTmux {
    bin: PathBuf,
    pub log: PathBuf,
    pub panes: PathBuf,
    pub clients: PathBuf,
    /// What `display-message` answers: where a window lives, as `session<TAB>group`.
    pub home: PathBuf,
    /// What `capture-pane` prints: the screen of every pane.
    pub screen: PathBuf,
}

impl FakeTmux {
    pub fn new(fixture: &Fixture) -> FakeTmux {
        let root = fixture._dir.path();
        let bin = root.join("fakebin");
        stub_bin(
            &bin,
            "tmux",
            "#!/bin/sh\n\
             echo \"$@\" >> \"$FAKE_TMUX_LOG\"\n\
             case \"$*\" in\n\
             -V) echo \"tmux 3.4\" ;;\n\
             *new-window*) [ -f \"$FAKE_TMUX_LOG.failnew\" ] && { echo \"no space for a new window\" >&2; exit 1; } ;;\n\
             *display-message*) cat \"$FAKE_TMUX_HOME\" ;;\n\
             *capture-pane*) cat \"$FAKE_TMUX_SCREEN\" ;;\n\
             *list-panes*) cat \"$FAKE_TMUX_PANES\" ;;\n\
             *list-clients*) cat \"$FAKE_TMUX_CLIENTS\" ;;\n\
             *kill-pane*|*kill-window*) [ -f \"$FAKE_TMUX_LOG.onkill\" ] && sh \"$FAKE_TMUX_LOG.onkill\"; [ -n \"$FAKE_TMUX_KILL\" ] && kill \"$FAKE_TMUX_KILL\" ;;\n\
             esac\n\
             exit 0\n",
        );
        let panes = root.join("panes.txt");
        std::fs::write(&panes, "").unwrap();
        let clients = root.join("clients.txt");
        std::fs::write(&clients, "").unwrap();
        let home = root.join("home.txt");
        std::fs::write(&home, "").unwrap();
        let screen = root.join("screen.txt");
        std::fs::write(&screen, "").unwrap();
        FakeTmux {
            bin,
            log: root.join("tmux.log"),
            panes,
            clients,
            home,
            screen,
        }
    }

    pub fn path(&self) -> String {
        path_with(&self.bin)
    }

    pub fn logged(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

pub fn write_tmux_config(fixture: &Fixture) {
    write_tmux_config_with(fixture, |_| {});
}

/// The tmux config, with whatever else the test needs put into it before it is written.
pub fn write_tmux_config_with(fixture: &Fixture, change: impl FnOnce(&mut serde_json::Value)) {
    let mut config = serde_json::json!({
        "notification": "true",
        "terminal": {"preset": "tmux", "session": "adjutant-test", "socket": "scratch"},
        "repos": {"acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"}},
    });
    change(&mut config);
    std::fs::write(&fixture.config, config.to_string()).unwrap();
}

pub fn resident_with_tmux(fixture: &Fixture, tmux: &FakeTmux, kill: Option<u32>) -> Resident {
    let path = tmux.path();
    let log = tmux.log.to_string_lossy().to_string();
    let panes = tmux.panes.to_string_lossy().to_string();
    let clients = tmux.clients.to_string_lossy().to_string();
    let home = tmux.home.to_string_lossy().to_string();
    let screen = tmux.screen.to_string_lossy().to_string();
    let kill = kill.map(|pid| pid.to_string()).unwrap_or_default();
    Resident::start_with(
        fixture,
        &[
            ("PATH", &path),
            ("FAKE_TMUX_LOG", &log),
            ("FAKE_TMUX_PANES", &panes),
            ("FAKE_TMUX_CLIENTS", &clients),
            ("FAKE_TMUX_HOME", &home),
            ("FAKE_TMUX_SCREEN", &screen),
            ("FAKE_TMUX_KILL", &kill),
        ],
    )
}

pub fn write_worker(worktree: &Path, started_at: &str, phase_at: Option<i64>) {
    std::fs::create_dir_all(worktree.join(".claude")).unwrap();
    let mut record = serde_json::json!({ "pid": 1, "startedAt": started_at, "phase": "implement" });
    if let Some(at) = phase_at {
        record["phaseAt"] = serde_json::json!(at);
    }
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
}

// 2026-09-22T05:00:00Z, well after the gates below are opened, and its stamp.
pub const LATER_SECS: i64 = 1_790_053_200;
pub const LATER_STAMP: &str = "20260922T050000Z";

pub fn sessions_url(path: &str) -> String {
    format!("/b/{SLUG}/api/sessions{path}")
}

pub fn state_of(resident: &Resident) -> serde_json::Value {
    let (status, body) = resident.get(&format!("/b/{SLUG}/api/state"));
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).unwrap()
}

/// A parent-task hub `FEATURE` known to the board only through its record, which is all a
/// session request or a link needs of it.
pub fn listed_parent_hub(fixture: &Fixture) {
    let record = fixture
        .state
        .join("hubs")
        .join(format!("{FEATURE_SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "hubName": FEATURE_HUB,
            "cwd": fixture.repo.to_str().unwrap(),
            "hub": FEATURE,
        })
        .to_string(),
    )
    .unwrap();
}

/// A linked worktree next to the repository, with the worker record and saved session a worker
/// that has started leaves in it. `pid` is the process it names: 1 for one nobody could take for
/// running, a `Sleeper` for one that is.
pub fn session_worktree(
    fixture: &Fixture,
    name: &str,
    hub: Option<&str>,
    task: Option<&str>,
    pid: u32,
) -> PathBuf {
    let worktree = fixture.repo.parent().unwrap().join(name);
    let out = Command::new("git")
        .hermetic()
        .args(["worktree", "add", "-q", "-b", name])
        .arg(&worktree)
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::create_dir_all(worktree.join(".claude")).unwrap();
    let mut record = serde_json::json!({
        "pid": pid,
        "psStarted": ps_started(pid),
        "title": name,
        "startedAt": "20260922T040000Z",
    });
    let mut session = serde_json::json!({"sessionId": "sid-1", "title": name});
    for (key, value) in [("hub", hub), ("task", task)] {
        if let Some(value) = value {
            record[key] = serde_json::json!(value);
            session[key] = serde_json::json!(value);
        }
    }
    std::fs::write(
        worktree.join(".claude").join("adjutant-worker.json"),
        record.to_string(),
    )
    .unwrap();
    std::fs::write(
        worktree.join(".claude").join("adjutant-session.json"),
        session.to_string(),
    )
    .unwrap();
    worktree
}

pub fn worker_record(worktree: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-worker.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

pub fn saved_session(worktree: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(worktree.join(".claude").join("adjutant-session.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// A task made through the board, as its record.
pub fn made_task(resident: &Resident, fields: serde_json::Value) -> serde_json::Value {
    let (status, body) = resident.post(&format!("/b/{SLUG}/api/tasks"), &fields.to_string());
    assert_eq!(status, 200, "{body}");
    serde_json::from_str::<serde_json::Value>(&body).unwrap()["task"].clone()
}

pub fn session_of<'a>(state: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    state["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .unwrap_or_else(|| panic!("no session {id} in {state}"))
}

/// Say in a worker's record where it runs, as `adj work` does when it starts one.
pub fn place_worker(worktree: &Path, window: &str) {
    let path = worktree.join(".claude").join("adjutant-worker.json");
    let mut record = worker_record(worktree);
    record["terminal"] =
        serde_json::json!({"backend": "tmux", "socket": "scratch", "window": window});
    std::fs::write(path, record.to_string()).unwrap();
}

pub fn write_gate_file(fixture: &Fixture, slug: &str, id: &str, kind: &str, worktree: &Path) {
    write_gate_file_with(fixture, slug, id, kind, worktree, serde_json::json!({}));
}

/// A gate file with more fields than the bare ones, merged in from `extra`.
pub fn write_gate_file_with(
    fixture: &Fixture,
    slug: &str,
    id: &str,
    kind: &str,
    worktree: &Path,
    extra: serde_json::Value,
) {
    let dir = fixture.state.join("gates").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    let mut gate = serde_json::json!({
        "id": id,
        "kind": kind,
        "worktree": worktree.to_str().unwrap(),
        "title": "どちらにするか",
        "openedAt": "20260922T041233Z",
    });
    for (key, value) in extra.as_object().unwrap() {
        gate[key] = value.clone();
    }
    std::fs::write(dir.join(format!("{id}.json")), gate.to_string()).unwrap();
}
