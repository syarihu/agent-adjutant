//! Which hub a command addresses: the name, the `--hub` identifier, and the record a
//! worktree carries.

mod common;

use common::*;

#[test]
fn the_hub_name_comes_from_the_remote_not_the_directory() {
    let fixture = Fixture::new(QUIET);
    assert_eq!(fixture.ok(&["hub-name"]).trim(), HUB);
    let info = fixture.json(&["hub-name", "--json"]);
    assert_eq!(info["nwo"], "acme/widget");
    assert_eq!(info["nwoSource"], "origin");
    assert_eq!(
        info["main"].as_str().unwrap(),
        fixture.repo.to_string_lossy()
    );
}

#[test]
fn a_worktree_answers_for_the_repository_it_belongs_to() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-1");
    let out = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            "wid-1",
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The hub lives in the main checkout, and a worker in a worktree has to reach the same
    // answer or its report goes to a different inbox.
    let from_worktree = fixture
        .command(["hub-name", "--json"])
        .current_dir(&worktree)
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&from_worktree.stdout).unwrap();
    assert_eq!(info["hubName"], HUB);
    assert_eq!(
        info["main"].as_str().unwrap(),
        fixture.repo.to_string_lossy()
    );
}

/// Git exports `GIT_DIR` to the hooks it runs, and a person can have one set. Honoured, it
/// would hand this repository's hub name, inbox and config entry to whichever checkout it
/// names.
#[test]
fn git_s_repository_location_variables_do_not_move_the_repository() {
    let fixture = Fixture::new(QUIET);
    let other = fixture.repo.parent().unwrap().join("other");
    std::fs::create_dir_all(&other).unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["remote", "add", "origin", "git@github.com:acme/other.git"],
    ] {
        let out = Command::new("git")
            .hermetic()
            .args(&args)
            .current_dir(&other)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let other_git = other.join(".git");

    // `GIT_WORK_TREE` is left to the unit test on `current_worktree`: nothing `hub-name`
    // prints goes through the one question it moves.
    for name in ["GIT_DIR", "GIT_COMMON_DIR"] {
        let out = fixture
            .command(["hub-name", "--json"])
            .env(name, &other_git)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let info: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(info["nwo"], "acme/widget", "{name}");
        assert_eq!(info["hubName"], HUB, "{name}");
        assert_eq!(
            info["main"].as_str().unwrap(),
            fixture.repo.to_string_lossy(),
            "{name}"
        );
    }
}

/// The compatibility lock, from the outside.
///
/// There are hub records, inboxes and archives sitting in `~/.local/state/adjutant` right
/// now, filed under the address a plain `adj` produced before any of this existed. An
/// upgrade that moved that address would leave every running hub unreachable and every
/// queued report unread, with nothing anywhere saying so — so the no-identifier answer has
/// to be the same bytes it always was, and the command line the launcher prints has to be
/// the same line it always printed.
#[test]
fn naming_no_hub_addresses_exactly_what_it_addressed_before() {
    let fixture = Fixture::new(QUIET);
    let info = fixture.json(&["hub-name", "--json"]);
    assert_eq!(info["hubName"], HUB);
    assert_eq!(info["slug"], SLUG);
    // Null rather than a string: nothing was asked for, and the repository's own hub is
    // not an identifier anybody typed.
    assert!(info["hub"].is_null(), "{info}");

    // The launcher's line gains no identifier. A hub that is the repository's own has none
    // to hand down, and an `env ADJUTANT_HUB=` prefix appearing here would be a change to a
    // command line people read, script and paste. (`ADJUTANT_HUB_SESSION` is a different
    // variable, and every hub carries one.)
    let launch = fixture.ok(&["hub", "--dry-run"]);
    assert!(!launch.contains("ADJUTANT_HUB="), "{launch}");
    assert!(launch.contains(&format!("claude -n {HUB}")), "{launch}");

    // Neither does the line that starts a worker.
    let work = fixture.ok(&[
        "work",
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!work.contains("--hub"), "{work}");
}

/// The point of the whole change: a second hub is a second *address*, not a second
/// repository.
///
/// Before this, the only way to ask for one was to invent a repository name — which moved
/// the address and emptied the configuration at the same time, because the config has no
/// entry for a repository that does not exist. The settings have to keep coming from
/// `owner/name` while the inbox moves.
#[test]
fn a_hub_identifier_moves_the_address_without_moving_the_configuration() {
    let fixture = Fixture::new(QUIET);
    let info = fixture.json(&["hub-name", "--hub", FEATURE, "--json"]);
    assert_eq!(info["hub"], FEATURE);
    assert_eq!(info["hubName"], FEATURE_HUB);
    assert_eq!(info["slug"], FEATURE_SLUG);
    assert_ne!(info["hubName"], HUB);
    // Still visibly this repository, because that is what a person picks out of a listing
    // of the state directory.
    assert_eq!(info["repo"], "widget");
    assert_eq!(info["nwo"], "acme/widget");

    // The half that must not move. `repos` is keyed by `owner/name`, and it is still
    // looked up by `owner/name`.
    let config = fixture.json(&["config", "--hub", FEATURE]);
    assert_eq!(config["registered"], true);
    assert_eq!(config["repo"], "acme/widget");
    assert_eq!(config["hub"], FEATURE);
    assert_eq!(config["hubName"], FEATURE_HUB);
    assert_eq!(config["config"]["taskSources"][0]["type"], "github");
    assert_eq!(config["config"]["issueKeys"]["acme/widget"], "WID");
    assert_eq!(config["config"]["verify"][0], "cargo test");

    // The half that must move: a report filed against one is not waiting in the other's
    // inbox. Both directories are asked for by path first, so a shared one fails here
    // rather than in the count below.
    let plain_dir = fixture.ok(&["pending", "--path"]).trim().to_string();
    let feature_dir = fixture
        .ok(&["pending", "--path", "--hub", FEATURE])
        .trim()
        .to_string();
    assert_ne!(plain_dir, feature_dir);
    assert!(feature_dir.contains(FEATURE_SLUG), "{feature_dir}");

    let sent = fixture.ok(&[
        "send",
        "--hub",
        FEATURE,
        "--from",
        "wid-957-worker",
        "--subject",
        "検索結果の画像が縦に潰れる",
        "--body",
        "b",
    ]);
    assert!(sent.contains(FEATURE_HUB), "{sent}");
    assert_eq!(
        fixture.json(&["pending", "--json", "--hub", FEATURE])["count"],
        1
    );
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // And a third identifier is a third address, not a shared one.
    assert_ne!(
        fixture.json(&["hub-name", "--hub", "wid-958", "--json"])["hubName"],
        info["hubName"]
    );
}

/// How a hub's own agent comes to know which hub it is.
///
/// Its procedure calls `adjutant_pending` and the rest with no arguments — it is talking
/// about itself. The identifier therefore has to reach those calls without any of them
/// mentioning it, and the environment is the one channel that does: the launcher puts it on
/// the line it `exec`s, the agent inherits it, and the MCP server the agent starts is that
/// agent's own child.
#[test]
fn the_launcher_hands_its_identifier_down_to_the_agent_it_starts() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--hub", FEATURE, "--dry-run"]);
    assert!(out.contains(&format!("ADJUTANT_HUB={FEATURE}")), "{out}");
    // The session is named for the address it answers at, or a worker looking for it finds
    // the repository's own hub instead.
    assert!(out.contains(&format!("claude -n {FEATURE_HUB}")), "{out}");
    assert!(
        out.contains(&fixture.repo.to_string_lossy().to_string()),
        "{out}"
    );

    // And the other way round: a command run *by* that agent, with no flag, addresses the
    // hub that started it. This is every `adj` call the hub's procedure makes.
    let inherited = fixture
        .command(["hub-name", "--json"])
        .env("ADJUTANT_HUB", FEATURE)
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&inherited.stdout).unwrap();
    assert_eq!(info["hubName"], FEATURE_HUB);
    assert_eq!(info["hub"], FEATURE);
}

/// A worker is dispatched with the hub that dispatched it, and finds it again without
/// being told.
///
/// The identifier cannot travel the way the hub's own does: the tab is opened by the
/// terminal, which is handed a command line and nothing else, so an inherited environment
/// does not survive the trip. It goes onto the command line instead, and from there into
/// the worktree — which is what lets `adj-report` keep promising that a worker never writes
/// an address down.
#[test]
fn a_worker_carries_the_hub_that_dispatched_it_into_its_worktree() {
    let fixture = Fixture::new(CODEX);
    let out = fixture.ok(&[
        "work",
        "--hub",
        FEATURE,
        "--worktree",
        fixture.repo.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(out.contains(&format!("--hub={FEATURE}")), "{out}");

    // A hub dispatching work runs this as its own child and passes no flag at all — it is
    // carrying the answer in its environment. The resolved identifier is what goes on the
    // line, not the flag, or exactly the hub that most needs this loses it.
    let inherited = fixture
        .command([
            "work",
            "--worktree",
            fixture.repo.to_str().unwrap(),
            "--dry-run",
        ])
        .env("ADJUTANT_HUB", FEATURE)
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&inherited.stdout);
    assert!(line.contains(&format!("--hub={FEATURE}")), "{line}");

    // The far end of that trip. `adj worker` writes the identifier into the worktree
    // before it becomes the agent; forged here rather than run, because running it would
    // `exec` an agent over this test.
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-957");
    let added = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            FEATURE,
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": 1, "title": "WID-957", "hub": FEATURE}).to_string(),
    )
    .unwrap();

    // No flag, no environment: the worker's agent says nothing about where it is sending,
    // and the report still reaches the hub that opened this worktree.
    let sent = fixture
        .command(["send", "--subject", "s", "--body", "b"])
        .current_dir(&worktree)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&sent.stdout);
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    assert!(said.contains(FEATURE_HUB), "{said}");
    assert_eq!(
        fixture.json(&["pending", "--json", "--hub", FEATURE])["count"],
        1
    );
    // And nothing landed in the repository's own inbox, which is the failure this whole
    // arrangement exists to prevent.
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);
}

/// A record left in a worktree says who dispatched *that* worktree. It must never be read
/// by the side that starts or registers a hub.
///
/// A worker that crashed without being closed leaves its record behind. Re-dispatching that
/// task under the repository's own hub would then file the new worker under the hub that ran
/// the old one, and every report it ever sends would go to an inbox that may have no hub
/// reading it — the silent misroute, arrived at from the other direction.
#[test]
fn a_record_left_in_a_worktree_never_decides_which_hub_is_being_started() {
    let fixture = Fixture::new(CODEX);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-957");
    let added = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            FEATURE,
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": 1, "title": "WID-957", "hub": FEATURE}).to_string(),
    )
    .unwrap();

    let from_worktree = |args: &[&str]| {
        let out = fixture
            .command(args)
            .current_dir(&worktree)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "adjutant {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    };

    // `adj hub` is run from anywhere in the repository, worktrees included. Typed here it
    // still starts the repository's own hub.
    let launch = from_worktree(&["hub", "--dry-run"]);
    assert!(launch.contains(&format!("claude -n {HUB}")), "{launch}");
    assert!(!launch.contains("ADJUTANT_HUB="), "{launch}");
    assert!(!launch.contains(FEATURE_HUB), "{launch}");

    // And the dispatching pair hands on its own identity rather than the worktree's. `adj
    // worker` matters most: its tab is opened *at* the worktree, so it would be reading the
    // record it is a moment from overwriting.
    let dispatched = from_worktree(&[
        "work",
        "--worktree",
        worktree.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!dispatched.contains("--hub"), "{dispatched}");

    // The other side of the asymmetry, and the reason it is not simply "ignore the record":
    // a command that *addresses* a hub from in here still reaches the one that dispatched
    // this worktree, which is what a worker's report depends on.
    let addressed: serde_json::Value =
        serde_json::from_str(&from_worktree(&["hub-name", "--json"])).unwrap();
    assert_eq!(addressed["hub"], FEATURE);
    assert_eq!(addressed["hubName"], FEATURE_HUB);
}

/// A record that cannot be read is not a record that says nothing.
///
/// Reading it as "nobody dispatched this worktree" addresses the repository's own hub, and
/// a worker whose worktree *was* dispatched then files every report it ever writes into an
/// inbox that may have no hub reading it. Both ends refuse instead: the CLI the worker's
/// agent types, and the MCP tools it calls.
#[test]
fn a_record_that_cannot_be_read_refuses_to_guess_which_hub_to_address() {
    let fixture = Fixture::new(QUIET);
    let worktree = fixture.repo.parent().unwrap().join("widget-wid-957");
    let added = Command::new("git")
        .hermetic()
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            FEATURE,
            worktree.to_str().unwrap(),
        ])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let record = worktree.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    // Cut off mid-string: a write that was interrupted leaves exactly this.
    std::fs::write(&record, r#"{"pid": 1, "title": "WID-957", "hub": "wid-9"#).unwrap();

    let from_worktree = |args: &[&str]| {
        fixture
            .command(args)
            .current_dir(&worktree)
            .output()
            .unwrap()
    };

    let refused = from_worktree(&["send", "--subject", "s", "--body", "b"]);
    assert_eq!(refused.status.code(), Some(1));
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(said.contains("worker record"), "{said}");
    // The assertion the fix is for: not filed under the repository's own hub instead.
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // The same refusal on the way in through the server, which is how a worker's agent
    // actually sends. `cwd` and not the server's own directory: one server answers about
    // whichever checkout the session is sitting in.
    let replies = mcp(
        &fixture,
        &[request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_send", "arguments": {
                "from": "wid-957-worker",
                "subject": "s",
                "body": "b",
                "cwd": worktree.to_string_lossy(),
            }}),
        )],
    );
    assert_eq!(replies[0]["result"]["isError"], true);
    let text = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("worker record"), "{text}");
    assert_eq!(fixture.json(&["pending", "--json"])["count"], 0);

    // And the way out, which is why this is an error rather than a dead end: what the
    // caller says outright is settled before the record is opened at all.
    let sent = from_worktree(&["send", "--hub", FEATURE, "--subject", "s", "--body", "b"]);
    assert!(
        sent.status.success(),
        "{}",
        String::from_utf8_lossy(&sent.stderr)
    );
    assert_eq!(
        fixture.json(&["pending", "--json", "--hub", FEATURE])["count"],
        1
    );

    // A command that never asks which hub is not stopped by the same record. `ide` and
    // `worktree-path` read the settings and the checkout and nothing else, and neither
    // takes `--hub` — so routing them through the addressing path would strand them with
    // advice they cannot take, on exactly the worktree somebody is trying to open.
    for args in [
        vec!["ide", "--worktree", worktree.to_str().unwrap(), "--dry-run"],
        vec!["worktree-path", "--name", "wid-957"],
    ] {
        let ran = from_worktree(&args);
        assert!(
            ran.status.success(),
            "{} stopped on a record it never reads: {}",
            args[0],
            String::from_utf8_lossy(&ran.stderr)
        );
    }
}

/// Every flag `work` hands the tab keeps its own description, and `--hub` did not steal one.
///
/// `--hub` was inserted between a description and the flag it described, so clap read the
/// pair as one doc comment on `--hub` and left the flag below it with none. Nothing fails
/// at runtime; the help text just stops explaining two flags.
#[test]
fn every_flag_the_help_lists_describes_itself_and_not_its_neighbour() {
    let fixture = Fixture::new(QUIET);
    let line = |help: &str, flag: &str| {
        help.lines()
            .find(|l| l.contains(flag))
            .unwrap_or_else(|| panic!("no {flag} in:\n{help}"))
            .to_string()
    };

    let work = fixture.ok(&["work", "--help"]);
    assert!(
        line(&work, "--hub <HUB>").contains("Which hub of the repository"),
        "{work}"
    );
    assert!(
        !line(&work, "--hub <HUB>").contains("What the worker is told"),
        "{work}"
    );
    assert!(
        line(&work, "--prompt <PROMPT>").contains("What the worker is told"),
        "{work}"
    );

    let focus = fixture.ok(&["focus", "--help"]);
    assert!(
        line(&focus, "--hub <HUB>").contains("Which hub of the repository"),
        "{focus}"
    );
    assert!(
        !line(&focus, "--hub <HUB>").contains("Say nothing"),
        "{focus}"
    );
    assert!(line(&focus, "--quiet").contains("Say nothing"), "{focus}");

    // And the help says which of the two answers the flag defaults to. A command that
    // *starts* something resolves the identifier without opening a worktree's record — a
    // hub launched from inside a worktree, and a worker registering in the one its tab was
    // opened at, would each read somebody else's. Promising the record there is a promise
    // the code deliberately breaks, and it is the direction of this change that it breaks.
    for command in ["hub", "work", "worker"] {
        let help = fixture.ok(&[command, "--help"]);
        assert!(
            !line(&help, "--hub <HUB>").contains("dispatched this worktree"),
            "{command} offers the worktree's record as its default, and never reads it:\n{help}"
        );
    }
    for command in [
        "send", "pending", "tell", "focus", "hub-stop", "hub-name", "config",
    ] {
        let help = fixture.ok(&[command, "--help"]);
        assert!(
            line(&help, "--hub <HUB>").contains("dispatched this worktree"),
            "{command} reads the worktree's record and does not say so:\n{help}"
        );
    }
}

/// An identifier that starts with a dash still reaches the worker.
///
/// It gets this far only through `ADJUTANT_HUB`, where no flag parser has seen it, and the
/// command line the terminal is handed has none between it and the worker either. As two
/// words, `--hub -x` reads as two options and the tab never starts; as one, it is a value.
#[test]
fn an_identifier_that_reads_like_a_flag_is_handed_down_as_one_argument() {
    let fixture = Fixture::new(CODEX);
    let inherited = fixture
        .command([
            "work",
            "--worktree",
            fixture.repo.to_str().unwrap(),
            "--dry-run",
        ])
        .env("ADJUTANT_HUB", "-x")
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&inherited.stdout);
    assert!(line.contains("--hub=-x"), "{line}");
    // And that spelling is one the far end takes: the same token, parsed.
    assert_eq!(
        fixture.json(&["hub-name", "--json", "--hub=-x"])["hub"],
        "-x"
    );
}

fn set_config(fixture: &Fixture, key: &str, value: serde_json::Value) {
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config[key] = value;
    std::fs::write(&fixture.config, config.to_string()).unwrap();
}

/// `agentEnv` may name the identifier, as a default. The command that starts an agent then
/// claims the hub the agent is going to address, rather than the repository's own.
///
/// Before, a plain `adj hub` claimed the repository's record, inbox and session name while
/// its agent was handed the configured identifier, and so read an inbox nobody was writing
/// to. And a worker, which never looked at the variable, registered under one hub while its
/// agent reported to the other.
#[test]
fn an_identifier_agent_env_names_is_the_one_the_launch_claims() {
    let fixture = Fixture::new(QUIET);
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config["repos"]["acme/widget"]["agentEnv"] = serde_json::json!({"ADJUTANT_HUB": FEATURE});
    std::fs::write(&fixture.config, config.to_string()).unwrap();
    let worktree = fixture.repo.to_str().unwrap();

    let launch = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        launch.contains(&format!("claude -n {FEATURE_HUB}")),
        "{launch}"
    );
    assert!(
        launch.contains(&format!("ADJUTANT_HUB={FEATURE} ")),
        "{launch}"
    );
    // Once: the configured assignment is replaced by the claimed one, not repeated.
    assert_eq!(launch.matches("ADJUTANT_HUB=").count(), 1, "{launch}");

    let work = fixture.ok(&["work", "--worktree", worktree, "--dry-run"]);
    assert!(work.contains(&format!("--hub={FEATURE}")), "{work}");

    let worker = fixture.ok(&["worker", "--worktree", worktree, "--dry-run"]);
    assert!(
        worker.contains(&format!("ADJUTANT_HUB={FEATURE} ")),
        "{worker}"
    );
    assert_eq!(worker.matches("ADJUTANT_HUB=").count(), 1, "{worker}");

    // A default, and nothing more: what was typed, and what the process was started with,
    // still outrank it — and the agent is handed that instead of the configured one.
    let flagged = fixture.ok(&["hub", "--hub", "wid-958", "--dry-run"]);
    let inherited = fixture
        .command(["hub", "--dry-run"])
        .env("ADJUTANT_HUB", "wid-958")
        .output()
        .unwrap();
    let inherited = String::from_utf8_lossy(&inherited.stdout).to_string();
    for line in [&flagged, &inherited] {
        assert!(line.contains("ADJUTANT_HUB=wid-958 "), "{line}");
        assert!(!line.contains(&format!("ADJUTANT_HUB={FEATURE}")), "{line}");
        assert!(!line.contains(FEATURE_HUB), "{line}");
    }
    let worker = fixture.ok(&[
        "worker",
        "--worktree",
        worktree,
        "--hub",
        "wid-958",
        "--dry-run",
    ]);
    assert!(worker.contains("ADJUTANT_HUB=wid-958 "), "{worker}");
    assert!(
        !worker.contains(&format!("ADJUTANT_HUB={FEATURE}")),
        "{worker}"
    );
}

/// A worker's agent reads the hub the worker registered under, whatever the tab inherited.
///
/// A terminal template that carries its environment into the tab — tmux does — hands the
/// agent the `ADJUTANT_HUB` of whichever hub ran `adj work`. That outranks the record in
/// the worktree, so every report would go to that hub instead of the one named on the line.
/// Run through to the exec rather than a dry run, because what matters is what the agent
/// actually sees.
#[test]
fn a_worker_hands_its_agent_the_hub_it_registered_under_not_the_one_it_inherited() {
    let fixture = Fixture::new(QUIET);
    let seen = fixture.repo.join("seen.txt");
    let echo = format!(
        "sh -c 'echo \"[$ADJUTANT_HUB]\"' > {} ; true {{sessionId}} {{prompt}}",
        shell_quoted(&seen.to_string_lossy())
    );
    set_config(&fixture, "agentRunner", echo.clone().into());
    set_config(&fixture, "agentResumeRunner", echo.into());
    let worktree = fixture.repo.to_str().unwrap();
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    let read = |path: &Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    };

    let out = fixture
        .command(["worker", "--worktree", worktree, "--hub", FEATURE])
        .env("ADJUTANT_HUB", "someone-else")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(&record)["hub"], FEATURE);
    assert_eq!(
        std::fs::read_to_string(&seen).unwrap().trim(),
        format!("[{FEATURE}]")
    );

    // A worker the repository's own hub dispatched records no identifier, and resumed from
    // a tab that inherited one, its agent must not be handed that one either.
    fixture.ok(&["worker", "--worktree", worktree]);
    let out = fixture
        .command(["worker", "--resume"])
        .env("ADJUTANT_HUB", "someone-else")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(&record).get("hub").is_none(), "{}", read(&record));
    assert_eq!(std::fs::read_to_string(&seen).unwrap().trim(), "[]");
}
