//! Starting a hub: the launcher, the tab route, the claim, and the flags carried down to the
//! agent.

mod common;

use common::*;

/// A hub record that is there and is not a record. Both readings of it — the one behind
/// `present` and the one behind a claim — have to meet it, so it is written as bytes rather
/// than as a JSON document with something wrong inside.
fn forge_unreadable_hub_record(fixture: &Fixture) -> PathBuf {
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "{ this was a hub record once").unwrap();
    record
}

/// A terminal that writes down what it was handed instead of opening anything, and a hub
/// runner that is `true`.
///
/// The runner matters even where the test never means to reach it: a regression that let a
/// refusal through would fall down to the exec path, and the net is what turns that into a
/// failing test rather than a real agent started inside the suite.
fn write_spawn_stub_config(fixture: &Fixture, spawned: &Path) {
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "hubRunner": "true {name} {prompt}",
            "terminal": {
                "spawn": format!(
                    "echo {{cwd}} {{command}} > {}",
                    shell_quoted(&spawned.to_string_lossy())
                ),
            },
            "repos": {
                "acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"},
            },
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn focus_exits_one_when_no_hub_is_running() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.cmd(&["focus", "--quiet"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn the_launcher_names_the_session_and_lands_in_the_main_checkout() {
    let fixture = Fixture::new(QUIET);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(
        out.contains(&fixture.repo.to_string_lossy().to_string()),
        "{out}"
    );
    assert!(out.contains(&format!("claude -n {HUB}")), "{out}");
    assert!(out.contains("--permission-mode auto"), "{out}");
    assert!(out.contains("adj-hub"), "{out}");
    // A dry run must not leave a record behind claiming a hub is up.
    assert_eq!(fixture.json(&["config"])["registered"], true);
    assert!(
        !fixture
            .state
            .join("hubs")
            .join(format!("{SLUG}.json"))
            .exists()
    );
}

/// Nothing could open a tab for a hub, so only a person at an empty one could start it.
///
/// `--tab` is the route for a caller that is not sitting in a spare tab — the same command,
/// opened rather than exec'd. Without it, `hub` is exactly what it was: this tab becomes the
/// hub, and no terminal is opened at all.
#[test]
fn a_hub_is_opened_in_a_new_tab_only_when_asked_for_one() {
    let fixture = Fixture::new(QUIET);

    // The default. It prints the agent command to run here, and never a command that would
    // open something: `adj hub` appearing in this output would mean the tab route leaked
    // into the one people already use.
    let here = fixture.ok(&["hub", "--dry-run"]);
    assert!(here.contains(&format!("claude -n {HUB}")), "{here}");
    assert!(!here.contains("/adjutant hub"), "{here}");

    // And the new one. The tab is handed the launcher, not the agent, for the same reason
    // `work` hands a tab `adjutant worker`: whoever ends up being the hub has to be the
    // process that wrote down its own PID.
    let tab = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(tab.contains("/adjutant hub"), "{tab}");
    assert!(!tab.contains(&format!("claude -n {HUB}")), "{tab}");
    // In the main checkout, which is where a hub has to be to cut a worktree at all.
    assert!(
        tab.contains(&fixture.repo.to_string_lossy().to_string()),
        "{tab}"
    );
    // The repository's own hub has no identifier to hand down, so nothing is added.
    assert!(!tab.contains("--hub"), "{tab}");
}

/// The side that opens the tab claims nothing.
///
/// A claim records the claiming process's PID, and the process that is going to be the hub
/// is the one in the new tab. Claiming here would name a launcher that exits a moment later,
/// and from then on every check would call a live hub gone and start another beside it.
///
/// Run for real rather than dry, because a dry run claims nothing either way and so could
/// not tell the two apart. The terminal is a stub that only writes down what it was handed.
#[test]
fn opening_a_tab_for_a_hub_leaves_the_claim_to_the_tab() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    // `{cwd}` is what tells `spawn` this template takes arguments rather than a shell line,
    // so `{command}` arrives as words for `echo` instead of a `cd … && …` chain that would
    // start a real hub inside the test suite.
    //
    // `hubRunner` is a no-op for the same reason it is here at all: the tab route never
    // reads it, so under test it changes nothing — but a regression that dropped the route
    // would fall through to the exec path, and this is what stops that from starting a real
    // agent inside the suite instead of failing.
    // Built with `json!` rather than by formatting a string: the redirect target has to be
    // shell-quoted, and a quoted path lands inside a JSON string — so the escaping of the two
    // has to be done by something that knows which is which.
    std::fs::write(
        &fixture.config,
        serde_json::json!({
            "notification": "true",
            "hubRunner": "true {name} {prompt}",
            "terminal": {
                "spawn": format!(
                    "echo {{cwd}} {{command}} > {}",
                    shell_quoted(&spawned.to_string_lossy())
                ),
            },
            "repos": {
                "acme/widget": {"taskSource": "github", "issueRepo": "acme/widget"},
            },
        })
        .to_string(),
    )
    .unwrap();

    let out = fixture.ok(&["hub", "--tab"]);
    assert!(out.contains("new tab"), "{out}");
    let handed = std::fs::read_to_string(&spawned).expect("the terminal was never asked");
    assert!(handed.contains("/adjutant hub"), "{handed}");
    assert!(
        handed.contains(&fixture.repo.to_string_lossy().to_string()),
        "{handed}"
    );
    // And the tab is handed the command *without* the flag, or it would open a tab of its
    // own, and that one another, and nothing would ever claim anything.
    assert!(!handed.contains("--tab"), "{handed}");

    // The record belongs to whoever ends up running the agent, and that is nobody yet.
    assert!(
        !fixture
            .state
            .join("hubs")
            .join(format!("{SLUG}.json"))
            .exists(),
        "the opening side claimed the hub: {handed}"
    );
}

/// One hub per address, whichever route is taken.
///
/// The presence check comes first and answers both routes the same way: bring the tab that
/// exists forward. Opening a second one would leave two sessions answering to one name, with
/// the record naming whichever of them claimed last.
#[test]
fn a_hub_that_is_already_running_is_brought_forward_rather_than_opened_again() {
    let fixture = Fixture::new(QUIET);
    // This very process stands in for the running hub, the same way the wake test does it:
    // both anchors have to be real, so the recorded name is this executable's and the start
    // time is the one the system reports for it.
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let written = serde_json::json!({
        "pid": std::process::id(), "hubName": name, "cwd": "/",
        "psStarted": ps_started(std::process::id()),
    })
    .to_string();
    std::fs::write(&record, &written).unwrap();

    let out = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(out.contains("is already running"), "{out}");
    assert!(!out.contains("/adjutant hub"), "{out}");
    // And the record it found is the record it leaves.
    assert_eq!(std::fs::read_to_string(&record).unwrap(), written);
}

/// The record is looked for where the hub keeps it, not where the command was typed.
///
/// A relative `ADJUTANT_STATE_DIR` resolves against the working directory, so a look taken
/// before the move to the main checkout reads a directory that holds nothing. The claim used
/// to cover for that by answering `Taken` a few lines later; the tab route never reaches the
/// claim, so a missed record there opens a tab for a hub that is already running.
#[test]
fn a_relative_state_directory_is_read_from_the_checkout_not_from_where_it_was_typed() {
    let fixture = Fixture::new(QUIET);
    let name = std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // Relative, and under the main checkout — which is the only place it resolves to the
    // same directory twice.
    let relative = "state-here";
    let record = fixture
        .repo
        .join(relative)
        .join("hubs")
        .join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({
            "pid": std::process::id(), "hubName": name, "cwd": "/",
            "psStarted": ps_started(std::process::id()),
        })
        .to_string(),
    )
    .unwrap();

    // Typed somewhere else inside the checkout, so a look taken before the move lands in a
    // directory that was never written to.
    let elsewhere = fixture.repo.join("somewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let out = Command::new(BIN)
        .args(["hub", "--tab", "--dry-run"])
        .current_dir(&elsewhere)
        .env("ADJUTANT_CONFIG", &fixture.config)
        .env("ADJUTANT_STATE_DIR", relative)
        .hermetic()
        .output()
        .unwrap();
    let said =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("is already running"), "{said}");
    assert!(!said.contains("/adjutant hub"), "{said}");
}

/// A hub the check could not match on is brought forward, not opened a second tab beside.
///
/// `hub_status` matches a record with no start time to anchor on by looking for the name in
/// the command line, and a hub that has replaced its own no longer carries it. The claim in
/// the tab would answer `Taken` and bring that hub forward — the same place, reached the
/// long way round, after this side has already said it started one and exited 0 for a hub
/// that was up the whole time.
#[test]
fn a_hub_the_presence_check_could_not_match_is_brought_forward_rather_than_opened_beside() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);

    // This process stands in for the hub: a live pid, under a name the check will not find
    // in its command line, and **no start time** — which is what sends the check to the
    // name and leaves the claim's own reading answering `Alive`.
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let written = serde_json::json!({
        "pid": std::process::id(), "hubName": "adjutant-acme-widget-renamed", "cwd": "/",
    })
    .to_string();
    std::fs::write(&record, &written).unwrap();

    let out = fixture.cmd(&["hub", "--tab"]);
    let said =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("is already running"), "{said}");
    assert!(!said.contains("new tab"), "{said}");
    assert!(
        !spawned.exists(),
        "a tab was opened for a hub that was already running"
    );
    // And the record it found is the record it leaves.
    assert_eq!(std::fs::read_to_string(&record).unwrap(), written);
}

/// A record whose liveness cannot be established is not a free name, and `--tab` is the one
/// route that would have taken it anyway.
///
/// `hub_status` answers `present: false` to "nobody is there" and to "cannot tell" alike.
/// The route that claims has `claim_hub` behind it to draw the line; the tab route leaves
/// the claim to the tab, so the line is drawn before the tab is opened or not at all — and
/// not at all is the worst of the two, because the caller `--tab` exists for is an agent
/// reading an exit code, and `exit 0` beside "started in a new tab" is read as a hub that
/// is now running.
#[test]
fn a_hub_whose_record_cannot_be_read_is_not_opened_in_a_tab() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);

    let record = forge_unreadable_hub_record(&fixture);
    let before = std::fs::read_to_string(&record).unwrap();

    // For real, not a dry run: a dry run opens nothing either way, so only this can tell a
    // refusal from a tab that was opened.
    let out = fixture.cmd(&["hub", "--tab"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("cannot tell whether the hub recorded in"),
        "{said}"
    );
    // Which record, so the person sent to look has somewhere to look.
    assert!(
        said.contains(&record.to_string_lossy().to_string()),
        "{said}"
    );
    assert!(
        !spawned.exists(),
        "a tab was opened for a hub nobody can account for"
    );
    // And the record it could not read is the record it leaves.
    assert_eq!(std::fs::read_to_string(&record).unwrap(), before);
}

/// The other way the answer goes missing: the record parses, and `ps` will not answer.
///
/// Two sources, one state. A fix that only asked whether the JSON parses would leave this
/// one opening tabs, and it is the likelier of the two on a machine under load or with a
/// locked-down `ps`.
#[test]
fn a_hub_is_not_opened_in_a_tab_when_ps_cannot_answer_for_the_record() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);

    // A perfectly good record, anchored on this process — asked before `ps` is taken away,
    // which is the same order the launcher wrote one in.
    let record = fixture.state.join("hubs").join(format!("{SLUG}.json"));
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let written = serde_json::json!({
        "pid": std::process::id(), "hubName": "adjutant-someone-else", "cwd": "/",
        "psStarted": ps_started(std::process::id()),
    })
    .to_string();
    std::fs::write(&record, &written).unwrap();

    // A `ps` that fails *with something on stderr*: that is what tells "I could not do
    // that" from "no such process", and only the second is an answer.
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let ps = stubs.join("ps");
    std::fs::write(
        &ps,
        "#!/bin/sh\necho 'ps: cannot do that here' >&2\nexit 1\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Prepended rather than replacing: the binary still has to find the real `git`.
    let path = format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = fixture
        .command(["hub", "--tab"])
        .env("PATH", &path)
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("cannot tell whether the hub recorded in"),
        "{said}"
    );
    assert!(
        said.contains(&record.to_string_lossy().to_string()),
        "{said}"
    );
    assert!(
        !spawned.exists(),
        "a tab was opened while `ps` was answering nothing"
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap(), written);
}

/// The route that claims refuses the same state, and refuses it the way it always did.
///
/// This is not new behaviour — `claim_hub` has always stopped here — and that is the point
/// of pinning it: the tab route was made to agree with this one, so a later change that
/// moved the words or the exit code would have the two disagreeing again.
#[test]
fn the_route_that_claims_refuses_an_unreadable_record_as_it_always_did() {
    let fixture = Fixture::new(QUIET);
    let spawned = fixture.repo.join("spawned.txt");
    write_spawn_stub_config(&fixture, &spawned);
    let record = forge_unreadable_hub_record(&fixture);

    // No `--tab`, and not a dry run, so this is the exec path — which `hubRunner: true`
    // keeps from starting a real agent if the refusal ever stops happening.
    let out = fixture.cmd(&["hub"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(
        said.contains("cannot tell whether the hub recorded in"),
        "{said}"
    );
    assert!(
        said.contains(&record.to_string_lossy().to_string()),
        "{said}"
    );
}

/// And the route that claims nothing still says what it would run.
///
/// A dry run on the default route prints a command and stops; it writes nothing, claims
/// nothing and opens nothing, so there is nothing there for an unreadable record to be
/// dangerous to. The refusal is the tab route's, and this is what says so: move it up a
/// line, to cover both routes, and this goes red.
#[test]
fn a_dry_run_that_claims_nothing_still_says_what_it_would_run() {
    let fixture = Fixture::new(QUIET);
    forge_unreadable_hub_record(&fixture);
    let out = fixture.ok(&["hub", "--dry-run"]);
    assert!(out.contains(&format!("claude -n {HUB}")), "{out}");
}

/// The tab is opened for one hub of the repository, and has to be told which.
///
/// The identifier cannot ride the environment across: the terminal is handed a command line
/// and nothing else. So it goes onto the line — and it is the *resolved* one, because the
/// caller this exists for is a hub opening a tab as its own child, passing no flag at all.
#[test]
fn a_tab_opened_for_a_hub_is_told_which_hub_it_is_opening() {
    let fixture = Fixture::new(QUIET);
    let asked = fixture.ok(&["hub", "--tab", "--hub", FEATURE, "--dry-run"]);
    assert!(asked.contains(&format!("--hub={FEATURE}")), "{asked}");
    // Named for the address it will answer at, so the tab is findable as that hub.
    assert!(asked.contains("adjutant-acme-widget-wid-957"), "{asked}");

    let inherited = fixture
        .command(["hub", "--tab", "--dry-run"])
        .env("ADJUTANT_HUB", FEATURE)
        .output()
        .unwrap();
    let line = String::from_utf8_lossy(&inherited.stdout);
    assert!(line.contains(&format!("--hub={FEATURE}")), "{line}");

    // Extra arguments survive the trip too: they are the trailing argument at both ends, so
    // the separator has to go back on the line the tab is handed.
    let extra = fixture.ok(&["hub", "--tab", "--dry-run", "--", "--resume"]);
    assert!(extra.contains("hub -- --resume"), "{extra}");
}

/// What `--no-dashboard` has to reach is the hub's own procedure, which asks the MCP server
/// for its settings — so the flag becomes a variable on the line the agent is `exec`ed
/// with, and the resolver folds it in before anybody reads `startupDashboard`.
#[test]
fn a_hub_told_not_to_collect_carries_that_down_to_the_agent_it_becomes() {
    let fixture = Fixture::new(QUIET);
    let off = fixture.ok(&["hub", "--dry-run", "--no-dashboard"]);
    assert!(off.contains("ADJUTANT_STARTUP_DASHBOARD=0"), "{off}");
    let on = fixture.ok(&["hub", "--dry-run", "--dashboard"]);
    assert!(on.contains("ADJUTANT_STARTUP_DASHBOARD=1"), "{on}");

    // Neither flag adds anything at all. The line a plain `adj hub` prints is one people
    // read, script and paste, and an override appearing in it would be a change to that
    // line for every user who never asked about the dashboard.
    let plain = fixture.ok(&["hub", "--dry-run"]);
    assert!(!plain.contains("ADJUTANT_STARTUP_DASHBOARD"), "{plain}");

    // Both at once is a contradiction with no sensible winner, so clap refuses it rather
    // than letting declaration order decide.
    let both = fixture.cmd(&["hub", "--dry-run", "--dashboard", "--no-dashboard"]);
    assert!(!both.status.success(), "{both:?}");
}

/// The `--tab` route never reaches the environment the other one builds: the terminal is
/// handed a command line and nothing else, exactly as with `--hub`. So the flag is
/// forwarded as a flag, and above the `--` — below it, clap at the far end would take it as
/// a trailing argument and append it to the *agent's* command instead of parsing it.
#[test]
fn a_tab_opened_for_a_hub_is_told_whether_to_collect_too() {
    let fixture = Fixture::new(QUIET);
    let off = fixture.ok(&["hub", "--tab", "--dry-run", "--no-dashboard"]);
    assert!(off.contains("--no-dashboard"), "{off}");
    // The variable belongs to the hub the tab starts, not to the launcher that opens it:
    // this line runs `adj hub`, and that invocation builds its own environment.
    assert!(!off.contains("ADJUTANT_STARTUP_DASHBOARD"), "{off}");

    let on = fixture.ok(&["hub", "--tab", "--dry-run", "--dashboard"]);
    assert!(on.contains("--dashboard"), "{on}");

    let extra = fixture.ok(&["hub", "--tab", "--dry-run", "--no-dashboard", "--", "-r"]);
    let line = extra.find("--no-dashboard").unwrap();
    assert!(line < extra.find("-- -r").unwrap(), "{extra}");

    let plain = fixture.ok(&["hub", "--tab", "--dry-run"]);
    assert!(!plain.contains("dashboard"), "{plain}");
}
