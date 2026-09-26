//! `adj jules`: a task's plan handed to a Jules session, with `curl` stubbed so nothing here
//! reaches the API, and a key command that prints a fixed key so the test can look for it.

mod common;

use common::*;

const KEY: &str = "sekrit-key-123";

fn config(jules_key: &str) -> String {
    format!(
        r#"{{"notification": "true", "julesKey": {jules_key},
            "repos": {{"acme/widget": {{"taskSource": "github", "issueRepo": "acme/widget",
                       "issueKeys": {{"acme/widget": "WID"}}}}}}}}"#
    )
}

/// A `curl` that writes down its arguments and its stdin, and answers like the API.
///
/// Creating answers session 42. Session 42 has opened a pull request. Anything else is a
/// 404 whose message repeats the header it was sent, which is what an error from a proxy
/// might do, so the test can see that the key is taken out of it.
fn stub_curl(fixture: &Fixture) -> (String, PathBuf, PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    std::fs::create_dir_all(&stubs).unwrap();
    let args = fixture.repo.join("curl-args");
    let stdin = fixture.repo.join("curl-stdin");
    let curl = stubs.join("curl");
    std::fs::write(
        &curl,
        format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$@\" > {args}\n\
             cat > {stdin}\n\
             for last; do :; done\n\
             case \"$*\" in\n\
             *'-X POST'*) sleep 1; printf '%s\\n200' '{{\"id\":\"42\",\"url\":\"https://jules.google.com/session/42\"}}' ;;\n\
             *) case \"$last\" in\n\
                *sessions/42) printf '%s\\n200' '{{\"id\":\"42\",\"state\":\"COMPLETED\",\"outputs\":[{{\"pullRequest\":{{\"url\":\"https://github.com/acme/widget/pull/7\"}}}}]}}' ;;\n\
                *) printf '{{\"error\":{{\"message\":\"no such session, you sent %s\"}}}}\\n404' \"$(cat {stdin})\" ;;\n\
                esac ;;\n\
             esac\n",
            args = shell_quoted(&args.to_string_lossy()),
            stdin = shell_quoted(&stdin.to_string_lossy()),
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).unwrap();
    // `adj jules start` asks `gh` who is signed in, and the real one would answer about the
    // developer running the suite.
    let gh = stubs.join("gh");
    if !gh.exists() {
        std::fs::write(
            &gh,
            "#!/bin/sh
case \"$1 $2\" in 'api user') echo someone ;; *) exit 1 ;; esac\n",
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        stubs.to_string_lossy(),
        std::env::var("PATH").unwrap_or_default()
    );
    (path, args, stdin)
}

fn add_task(fixture: &Fixture, extra: &[&str]) -> String {
    let mut args = vec![
        "task",
        "add",
        "--title",
        "Add a setup section",
        "--body",
        "b",
        "--json",
    ];
    args.extend_from_slice(extra);
    let added = fixture.json(&args);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    // In progress, as a task is by the time its worker hands it to Jules.
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--status",
        "dispatched",
        "--executor",
        "jules",
        "--no-hand-over",
    ]);
    id
}

fn write_prompt(fixture: &Fixture) -> String {
    let prompt = fixture.repo.join("prompt.md");
    std::fs::write(&prompt, "Edit README.md only.\nAdd a \"Setup\" section.\n").unwrap();
    prompt.to_string_lossy().to_string()
}

#[test]
fn a_started_session_is_written_onto_the_task_and_the_key_only_goes_to_stdin() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let id = add_task(&fixture, &["--executor", "jules"]);
    let prompt = write_prompt(&fixture);
    let (path, args, stdin) = stub_curl(&fixture);

    let out = fixture
        .command([
            "jules",
            "start",
            "--id",
            &id,
            "--prompt-file",
            &prompt,
            "--base",
            "main",
            "--json",
        ])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let started: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(started["session"]["id"], "42");

    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["julesSession"], "42");
    assert_eq!(shown["executor"], "jules");
    assert_eq!(shown["julesBy"], "someone");

    let sent_args = std::fs::read_to_string(&args).unwrap();
    assert!(
        !sent_args.contains(KEY),
        "the key was on curl's command line"
    );
    assert!(sent_args.contains("https://jules.googleapis.com/v1alpha/sessions"));
    let body_line = sent_args
        .lines()
        .find(|l| l.starts_with('{'))
        .expect("a JSON body");
    let body: serde_json::Value = serde_json::from_str(body_line).unwrap();
    assert_eq!(
        body["sourceContext"]["source"],
        "sources/github/acme/widget"
    );
    assert_eq!(
        body["sourceContext"]["githubRepoContext"]["startingBranch"],
        "main"
    );
    assert_eq!(body["automationMode"], "AUTO_CREATE_PR");
    assert!(
        body["prompt"]
            .as_str()
            .unwrap()
            .contains("\"Setup\" section")
    );
    assert_eq!(
        std::fs::read_to_string(&stdin).unwrap(),
        format!("x-goog-api-key: {KEY}\n")
    );
}

#[test]
fn a_task_already_with_jules_is_not_handed_over_twice() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let id = add_task(&fixture, &["--base", "main"]);
    fixture.ok(&["task", "update", "--id", &id, "--jules-session", "41"]);
    let prompt = write_prompt(&fixture);
    let (path, args, _) = stub_curl(&fixture);

    let out = fixture
        .command(["jules", "start", "--id", &id, "--prompt-file", &prompt])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already with Jules"));
    assert!(!args.exists(), "the API was called anyway");
}

#[test]
fn the_base_comes_from_the_task_when_none_is_given_and_is_asked_for_otherwise() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let prompt = write_prompt(&fixture);
    let (path, args, _) = stub_curl(&fixture);

    let without = add_task(&fixture, &[]);
    let out = fixture
        .command(["jules", "start", "--id", &without, "--prompt-file", &prompt])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--base"));

    let with = add_task(&fixture, &["--base", "release/2.0"]);
    let out = fixture
        .command(["jules", "start", "--id", &with, "--prompt-file", &prompt])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        std::fs::read_to_string(&args)
            .unwrap()
            .contains("release/2.0")
    );
}

#[test]
fn a_session_is_shown_with_its_pull_request() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let id = add_task(&fixture, &[]);
    fixture.ok(&["task", "update", "--id", &id, "--jules-session", "42"]);
    let (path, _, _) = stub_curl(&fixture);

    let out = fixture
        .command(["jules", "show", "--id", &id, "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let session: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(session["state"], "COMPLETED");
    assert_eq!(session["pr"], "https://github.com/acme/widget/pull/7");
}

#[test]
fn an_error_from_the_api_is_reported_with_the_key_taken_out() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let (path, _, _) = stub_curl(&fixture);

    let out = fixture
        .command(["jules", "show", "--session", "404"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        said.contains("404") && said.contains("no such session"),
        "{said}"
    );
    assert!(!said.contains(KEY), "the key leaked into the error: {said}");
}

#[test]
fn a_jules_key_turned_off_stops_before_anything_is_sent() {
    let fixture = Fixture::new(&config("false"));
    let (path, args, _) = stub_curl(&fixture);

    let out = fixture
        .command(["jules", "show", "--session", "42"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("julesKey is false"));
    assert!(!args.exists());
}

#[test]
fn a_jules_task_says_so_in_the_request_the_hub_reads_and_a_typo_is_refused() {
    let fixture = Fixture::new(&config("false"));
    add_task(&fixture, &["--executor", "jules", "--queue"]);
    let listed = fixture.json(&["pending", "--json"]);
    let name = listed["messages"][0]["name"].as_str().unwrap().to_string();
    let pending = fixture.ok(&["pending", "--read", &name]);
    assert!(pending.contains("## 実装        jules"), "{pending}");

    let out = fixture.cmd(&[
        "task",
        "add",
        "--title",
        "t",
        "--body",
        "b",
        "--executor",
        "julse",
    ]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no such executor"));
}

#[test]
fn a_stored_base_written_for_git_worktree_is_handed_to_jules_as_the_branch_name() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    // What `git fetch` would leave: a remote-tracking ref for the branch.
    let out = Command::new("git")
        .hermetic()
        .args(["update-ref", "refs/remotes/origin/release/2.0", "HEAD"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let prompt = write_prompt(&fixture);
    let (path, args, _) = stub_curl(&fixture);
    let body_sent = || {
        let sent = std::fs::read_to_string(&args).unwrap();
        let line = sent
            .lines()
            .find(|l| l.starts_with('{'))
            .unwrap()
            .to_string();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()
    };

    let stored = add_task(&fixture, &["--base", "origin/release/2.0"]);
    let out = fixture
        .command(["jules", "start", "--id", &stored, "--prompt-file", &prompt])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        body_sent()["sourceContext"]["githubRepoContext"]["startingBranch"],
        "release/2.0"
    );

    // Given explicitly, it is taken as written.
    let given = add_task(&fixture, &[]);
    let out = fixture
        .command([
            "jules",
            "start",
            "--id",
            &given,
            "--prompt-file",
            &prompt,
            "--base",
            "origin/x",
        ])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        body_sent()["sourceContext"]["githubRepoContext"]["startingBranch"],
        "origin/x"
    );
}

#[test]
fn a_stored_base_is_left_as_written_when_this_checkout_is_another_repository() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let out = Command::new("git")
        .hermetic()
        .args(["update-ref", "refs/remotes/origin/release/2.0", "HEAD"])
        .current_dir(&fixture.repo)
        .output()
        .unwrap();
    assert!(out.status.success());
    let prompt = write_prompt(&fixture);
    let (path, args, _) = stub_curl(&fixture);
    let id = fixture.json(&[
        "task",
        "add",
        "--repo",
        "acme/other",
        "--title",
        "t",
        "--body",
        "b",
        "--base",
        "origin/release/2.0",
        "--json",
    ])["task"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    fixture.ok(&[
        "task",
        "update",
        "--repo",
        "acme/other",
        "--id",
        &id,
        "--status",
        "dispatched",
        "--executor",
        "jules",
        "--no-hand-over",
    ]);
    let out = fixture
        .command([
            "jules",
            "start",
            "--repo",
            "acme/other",
            "--id",
            &id,
            "--prompt-file",
            &prompt,
        ])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sent = std::fs::read_to_string(&args).unwrap();
    let body: serde_json::Value =
        serde_json::from_str(sent.lines().find(|l| l.starts_with('{')).unwrap()).unwrap();
    assert_eq!(body["sourceContext"]["source"], "sources/github/acme/other");
    assert_eq!(
        body["sourceContext"]["githubRepoContext"]["startingBranch"],
        "origin/release/2.0"
    );
}

#[test]
fn a_task_not_in_progress_is_not_handed_to_jules() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let added = fixture.json(&[
        "task", "add", "--title", "t", "--body", "b", "--base", "main", "--json",
    ]);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    let prompt = write_prompt(&fixture);
    let (path, args, _) = stub_curl(&fixture);
    let out = fixture
        .command(["jules", "start", "--id", &id, "--prompt-file", &prompt])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not in progress"));
    assert!(!args.exists(), "the API was called anyway");
}

#[test]
fn a_task_its_worker_implements_is_not_handed_to_jules() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let id = add_task(&fixture, &["--base", "main"]);
    fixture.ok(&["task", "update", "--id", &id, "--executor", "worker"]);
    let prompt = write_prompt(&fixture);
    let (path, args, _) = stub_curl(&fixture);
    let out = fixture
        .command(["jules", "start", "--id", &id, "--prompt-file", &prompt])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("implemented by its worker"));
    assert!(!args.exists(), "the API was called anyway");
}

#[test]
fn two_starts_for_one_task_at_once_create_one_session() {
    let fixture = Fixture::new(&config(&format!("\"echo {KEY}\"")));
    let id = add_task(&fixture, &["--base", "main"]);
    let prompt = write_prompt(&fixture);
    let (path, _, _) = stub_curl(&fixture);
    let spawn = || {
        fixture
            .command(["jules", "start", "--id", &id, "--prompt-file", &prompt])
            .env("PATH", &path)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let (a, b) = (spawn(), spawn());
    let outs = [a.wait_with_output().unwrap(), b.wait_with_output().unwrap()];
    let ok = outs.iter().filter(|o| o.status.success()).count();
    assert_eq!(ok, 1, "exactly one start should have created a session");
    let refused = outs.iter().find(|o| !o.status.success()).unwrap();
    assert!(String::from_utf8_lossy(&refused.stderr).contains("already with Jules"));
}
