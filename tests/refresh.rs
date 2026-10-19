//! `adj task refresh` and `adjutant_refresh`: records brought up to date with their pull
//! requests, with `gh` stubbed so nothing here reaches GitHub.

mod common;

use common::*;

const PULL: &str = "https://github.com/acme/widget/pull";

/// One pull request as the refresh's GraphQL query answers it.
///
/// GitHub's upper-case values are passed in rather than written into the JSON: a bare
/// upper-case string after a colon reads as a tracker key to the guard over everything that
/// ships. `checks` are (status, conclusion) pairs of check runs, counted by state the way the
/// query asks for them; `review` is empty for no decision yet.
fn pr_json(state: &str, draft: bool, review: &str, checks: &[(&str, &str)]) -> String {
    let mut counts: std::collections::BTreeMap<&str, u32> = Default::default();
    for (status, conclusion) in checks {
        let state = if *status == "COMPLETED" {
            conclusion
        } else {
            status
        };
        *counts.entry(state).or_default() += 1;
    }
    let runs: Vec<String> = counts
        .iter()
        .map(|(state, count)| format!(r#"{{"state":"{state}","count":{count}}}"#))
        .collect();
    // Somebody is only waited on when GitHub says a review is required.
    let asked = u32::from(review == "REVIEW_REQUIRED");
    let review = if review.is_empty() {
        "null".to_string()
    } else {
        format!("\"{review}\"")
    };
    format!(
        r#"{{"state":"{state}","isDraft":{draft},"title":"A pull request","reviewDecision":{review},"reviewRequests":{{"totalCount":{asked}}},"latestOpinionatedReviews":{{"nodes":[]}},"commits":{{"nodes":[{{"commit":{{"statusCheckRollup":{{"contexts":{{"checkRunCountsByState":[{}],"statusContextCountsByState":[]}}}}}}}}]}}}}"#,
        runs.join(",")
    )
}

/// What `gh api graphql` prints for one pull request, `p0`.
fn answer_for(pr: &str) -> String {
    format!(r#"{{"data":{{"p0":{{"pullRequest":{pr}}}}}}}"#)
}

/// A `gh` that knows four pull requests, and writes down the number of every one it is asked
/// about, and every time it is run.
///
/// 1 is merged, 2 is open, 3 was closed without merging, 5 is merged but its record is
/// removed while `gh` is answering, and anything else fails the way `gh` does for a PR it
/// cannot find: an answer with the errors in it, and a non-zero exit.
fn stub_gh(fixture: &Fixture) -> (String, PathBuf) {
    let stubs = fixture.repo.join("stub-bin");
    let asked = fixture.repo.join("gh-asked");
    let calls = fixture.repo.join("gh-calls");
    // `-F pK=N` is how the query is given each pull request's number.
    let script = r#"#!/bin/sh
echo call >> @CALLS@
data=''
errors=''
for a in "$@"; do
  case "$a" in
    p[0-9]*=*)
      alias=${a%%=*}
      n=${a#*=}
      echo "$n" >> @ASKED@
      case "$n" in
        1) pr='@MERGED@' ;;
        2) pr='@OPEN@' ;;
        3) pr='@CLOSED@' ;;
        5) grep -l '/pull/5"' "$ADJUTANT_STATE_DIR"/tasks/*/*.json | xargs rm; pr='@MERGED@' ;;
        *) pr=null
           errors="$errors{\"type\":\"NOT_FOUND\",\"path\":[\"$alias\",\"pullRequest\"],\"message\":\"Could not resolve to a PullRequest with the number of $n.\"}," ;;
      esac
      data="$data\"$alias\":{\"pullRequest\":$pr},"
      ;;
  esac
done
data=${data%,}
errors=${errors%,}
if [ -n "$errors" ]; then
  echo "{\"data\":{$data},\"errors\":[$errors]}"
  echo 'gh: Could not resolve to a PullRequest' >&2
  exit 1
fi
echo "{\"data\":{$data}}"
"#
    .replace("@CALLS@", &shell_quoted(&calls.to_string_lossy()))
    .replace("@ASKED@", &shell_quoted(&asked.to_string_lossy()))
    .replace("@MERGED@", &pr_json("MERGED", false, "", &[]))
    .replace("@OPEN@", &pr_json("OPEN", false, "", &[]))
    .replace("@CLOSED@", &pr_json("CLOSED", false, "", &[]));
    stub_bin(&stubs, "gh", &script);
    // Prepended rather than replacing: the binary still has to find the real `git`.
    let path = path_with(&stubs);
    (path, asked)
}

/// A directory with only what the binary needs besides `gh`: `git`, linked into a directory
/// of its own. Not the directory `git` lives in, which is where `gh` is often installed too — on
/// a CI runner both are in /usr/bin — and a PATH through it would still find `gh`.
fn git_only_bin(fixture: &Fixture) -> PathBuf {
    let git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let git = PathBuf::from(String::from_utf8_lossy(&git.stdout).trim());
    let only_git = fixture.repo.join("git-only-bin");
    std::fs::create_dir_all(&only_git).unwrap();
    let _ = std::fs::remove_file(only_git.join("git"));
    std::os::unix::fs::symlink(&git, only_git.join("git")).unwrap();
    only_git
}

/// A record in `status`, pointing at `pr`. Written with no `gh` on the PATH: moving a record
/// that has a PR to done reads it, and a test must never reach the developer's own `gh`.
fn task_with_pr(fixture: &Fixture, title: &str, status: &str, pr: &str) -> String {
    let added = fixture.json(&["task", "add", "--title", title, "--body", title, "--json"]);
    let id = added["task"]["id"].as_str().unwrap().to_string();
    let out = fixture
        .command([
            "task",
            "update",
            "--id",
            &id,
            "--status",
            status,
            "--pr",
            pr,
            "--no-hand-over",
        ])
        .env("PATH", git_only_bin(fixture))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    id
}

fn status_of(fixture: &Fixture, id: &str) -> String {
    let shown = fixture.json(&["task", "show", "--id", id]);
    shown["status"].as_str().unwrap().to_string()
}

fn ids(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn only_a_merged_pr_moves_its_task_to_done_and_the_rest_are_reported() {
    let fixture = Fixture::new(QUIET);
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let open = task_with_pr(&fixture, "open", "pr", &format!("{PULL}/2"));
    let closed = task_with_pr(&fixture, "closed", "pr", &format!("{PULL}/3"));
    let missing = task_with_pr(&fixture, "missing", "pr", &format!("{PULL}/4"));
    // Still being worked on, but its PR is already in: merged is merged.
    let working = task_with_pr(&fixture, "working", "dispatched", &format!("{PULL}/1"));
    let (path, _) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();

    let mut done = ids(&result["done"]);
    done.sort();
    let mut expected = vec![merged.clone(), working.clone()];
    expected.sort();
    assert_eq!(done, expected, "{result}");
    assert_eq!(ids(&result["open"]), [open.as_str()], "{result}");
    assert_eq!(ids(&result["closed"]), [closed.as_str()], "{result}");
    // Whose turn each is, for the caller to say; a closed PR is the person's to decide.
    assert_eq!(result["closed"][0]["turn"], "closed", "{result}");
    assert_eq!(result["open"][0]["turn"], "unrequested", "{result}");
    assert_eq!(ids(&result["unreadable"]), [missing.as_str()], "{result}");
    assert!(
        result["unreadable"][0]["error"]
            .as_str()
            .unwrap()
            .contains("Could not resolve"),
        "{result}"
    );

    assert_eq!(status_of(&fixture, &merged), "done");
    assert_eq!(status_of(&fixture, &working), "done");
    for id in [&open, &closed, &missing] {
        assert_eq!(status_of(&fixture, id), "pr", "{id} was moved");
    }
}

/// Many records in one query: each answer still lands on its own record.
#[test]
fn every_record_gets_its_own_answer_out_of_one_query() {
    let fixture = Fixture::new(QUIET);
    let prs = [1, 2, 3, 4];
    let made: Vec<(String, i32)> = (0..11)
        .map(|n| {
            let pr = prs[n % prs.len()];
            let id = task_with_pr(&fixture, &format!("t{n}"), "pr", &format!("{PULL}/{pr}"));
            (id, pr)
        })
        .collect();
    let (path, _) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    // All of them in one query, however many there are.
    let calls = std::fs::read_to_string(fixture.repo.join("gh-calls")).unwrap();
    assert_eq!(calls.lines().count(), 1, "{calls}");
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for (id, pr) in &made {
        let bucket = match pr {
            1 => "done",
            2 => "open",
            3 => "closed",
            _ => "unreadable",
        };
        assert!(
            ids(&result[bucket]).contains(id),
            "{id} is not in {bucket}: {result}"
        );
        let want = if *pr == 1 { "done" } else { "pr" };
        assert_eq!(status_of(&fixture, id), want, "{id}");
    }
}

/// One record that cannot be moved does not take the others with it: the ones that were
/// moved are still reported, and the one that failed says why.
#[test]
fn a_record_that_cannot_be_moved_is_reported_and_the_rest_still_move() {
    let fixture = Fixture::new(QUIET);
    let gone = task_with_pr(&fixture, "gone", "pr", &format!("{PULL}/5"));
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let (path, _) = stub_gh(&fixture);

    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ids(&result["done"]), [merged.as_str()], "{result}");
    assert_eq!(ids(&result["failed"]), [gone.as_str()], "{result}");
    assert!(
        result["failed"][0]["error"]
            .as_str()
            .unwrap()
            .contains("no such task"),
        "{result}"
    );
    assert!(ids(&result["skipped"]).is_empty(), "{result}");
    assert_eq!(status_of(&fixture, &merged), "done");
}

/// A cancelled record is not looked up at all, and a done one only until its PR state is
/// final: a merged or closed PR cannot change.
#[test]
fn a_cancelled_task_and_a_done_one_recorded_final_are_not_asked_about() {
    let fixture = Fixture::new(QUIET);
    let cancelled = task_with_pr(&fixture, "cancelled", "cancelled", &format!("{PULL}/1"));
    let (path, asked) = stub_gh(&fixture);
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let closed = task_with_pr(&fixture, "closed", "pr", &format!("{PULL}/3"));
    // Their PRs' states are read once, as the moves to done read them.
    for id in [&merged, &closed] {
        let out = fixture
            .command(["task", "update", "--id", id, "--status", "done"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let _ = std::fs::remove_file(&asked);

    let out = fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("No task is waiting on a pull request"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!asked.exists(), "gh was asked about a finished task");
    assert_eq!(status_of(&fixture, &cancelled), "cancelled");
    let shown = fixture.json(&["task", "show", "--id", &closed]);
    assert_eq!(shown["prStatus"]["state"], "closed", "{shown}");
}

/// A `gh` that answers `answer_file`'s content for the pull request it is asked about, and
/// counts its runs in the file it returns.
fn counting_gh(fixture: &Fixture, answer_file: &Path) -> (String, PathBuf) {
    let stubs = fixture.repo.join("counting-bin");
    let calls = fixture.repo.join("counting-calls");
    stub_bin(
        &stubs,
        "gh",
        &format!(
            "#!/bin/sh\necho call >> {}\ncat {}\n",
            shell_quoted(&calls.to_string_lossy()),
            shell_quoted(&answer_file.to_string_lossy())
        ),
    );
    (path_with(&stubs), calls)
}

fn set_answer(file: &Path, state: &str) {
    std::fs::write(file, answer_for(&pr_json(state, false, "", &[]))).unwrap();
}

fn run_with(fixture: &Fixture, path: &str, args: &[&str]) -> std::process::Output {
    let out = fixture.command(args).env("PATH", path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn call_count(calls: &Path) -> usize {
    std::fs::read_to_string(calls).map_or(0, |t| t.lines().count())
}

/// A card moved to done while its PR was still open keeps `open` until a refresh reads it
/// again: then the state is corrected, and the record is not moved, not stamped as just
/// finished, and not listed. Once merged it is not asked about again.
#[test]
fn a_done_task_recorded_open_is_corrected_without_being_moved() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "work", "pr", &format!("{PULL}/9"));
    let answer = fixture.repo.join("answer");
    set_answer(&answer, "OPEN");
    let (path, calls) = counting_gh(&fixture, &answer);
    run_with(&fixture, &path, &["task", "refresh"]);
    run_with(
        &fixture,
        &path,
        &["task", "update", "--id", &id, "--status", "done"],
    );
    let before = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(before["status"], "done");
    assert_eq!(before["prStatus"]["state"], "open", "{before}");

    set_answer(&answer, "MERGED");
    let out = run_with(&fixture, &path, &["task", "refresh", "--json"]);
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for list in ["done", "open", "skipped", "unreadable", "closed"] {
        if let Some(entries) = result.get(list) {
            assert!(ids(entries).is_empty(), "{list}: {result}");
        }
    }
    let after = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(after["status"], "done");
    assert_eq!(after["prStatus"]["state"], "merged", "{after}");
    assert_eq!(after["updatedAt"], before["updatedAt"], "{after}");
    assert_eq!(after["prTurnAt"], before["prTurnAt"], "{after}");

    let asked = call_count(&calls);
    run_with(&fixture, &path, &["task", "refresh"]);
    assert_eq!(call_count(&calls), asked, "a merged record was asked about");
}

/// A done task whose PR is still open is read at every refresh, and stays done.
#[test]
fn a_done_task_whose_pr_is_still_open_stays_done() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "work", "pr", &format!("{PULL}/9"));
    let answer = fixture.repo.join("answer");
    set_answer(&answer, "OPEN");
    let (path, calls) = counting_gh(&fixture, &answer);
    run_with(
        &fixture,
        &path,
        &["task", "update", "--id", &id, "--status", "done"],
    );
    let asked = call_count(&calls);
    run_with(&fixture, &path, &["task", "refresh"]);
    assert!(call_count(&calls) > asked, "the done task was not read");
    assert_eq!(status_of(&fixture, &id), "done");
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["prStatus"]["state"], "open", "{shown}");
}

/// Moving a card to done reads its PR once, so a PR merged before the move is recorded as
/// merged; the turn is not stamped, as the record never saw the change happen.
#[test]
fn moving_a_task_with_a_pr_to_done_records_its_pr_state() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "work", "pr", &format!("{PULL}/9"));
    let answer = fixture.repo.join("answer");
    set_answer(&answer, "MERGED");
    let (path, _) = counting_gh(&fixture, &answer);
    run_with(
        &fixture,
        &path,
        &["task", "update", "--id", &id, "--status", "done"],
    );
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["status"], "done");
    assert_eq!(shown["prStatus"]["state"], "merged", "{shown}");
    assert!(shown["prTurnAt"].is_null(), "{shown}");
}

/// A `gh` that cannot be run does not stop the move.
#[test]
fn moving_to_done_without_gh_still_moves() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "work", "pr", &format!("{PULL}/9"));
    let only_git = git_only_bin(&fixture);
    run_with(
        &fixture,
        &only_git.to_string_lossy(),
        &["task", "update", "--id", &id, "--status", "done"],
    );
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["status"], "done");
    assert!(shown["prStatus"].is_null(), "{shown}");
}

/// Without `gh` there is no answer, and no answer is not "merged".
#[test]
fn without_gh_every_task_is_left_alone_and_said_to_be_unreadable() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let only_git = git_only_bin(&fixture);
    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &only_git)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ids(&result["unreadable"]), [id.as_str()], "{result}");
    assert_eq!(status_of(&fixture, &id), "pr");
}

#[test]
fn the_tool_does_what_the_command_does() {
    let fixture = Fixture::new(QUIET);
    let merged = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let open = task_with_pr(&fixture, "open", "pr", &format!("{PULL}/2"));
    let (path, _) = stub_gh(&fixture);

    let mut child = Reaped(
        fixture
            .command(["mcp"])
            .env("PATH", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        request(
            1,
            "tools/call",
            serde_json::json!({"name": "adjutant_refresh", "arguments": {
                "cwd": fixture.repo.to_str().unwrap(),
            }}),
        )
    )
    .unwrap();
    let out = child.wait_with_output();
    let reply: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).lines().next().unwrap()).unwrap();
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    let result: serde_json::Value = serde_json::from_str(text).expect(text);

    assert_eq!(ids(&result["done"]), [merged.as_str()], "{result}");
    assert_eq!(ids(&result["open"]), [open.as_str()], "{result}");
    assert_eq!(status_of(&fixture, &merged), "done");
    assert_eq!(status_of(&fixture, &open), "pr");
}

/// A `gh` that answers with `answer` as the pull request it is asked about, read from a file so that an answer
/// longer than a pipe holds is no trouble to write.
fn stub_gh_answering(fixture: &Fixture, answer: &str) -> String {
    let stubs = fixture.repo.join("stub-bin");
    let said = fixture.repo.join("gh-answer");
    std::fs::write(&said, answer_for(answer)).unwrap();
    stub_bin(
        &stubs,
        "gh",
        &format!("#!/bin/sh\ncat {}\n", shell_quoted(&said.to_string_lossy())),
    );
    path_with(&stubs)
}

fn record_file(fixture: &Fixture, id: &str) -> PathBuf {
    std::fs::read_dir(fixture.state.join("tasks"))
        .unwrap()
        .flatten()
        .map(|slug| slug.path().join(format!("{id}.json")))
        .find(|p| p.exists())
        .expect("the record's file")
}

/// What the card shows about a PR is read by the refresh and kept on the record, so the page
/// does not ask `gh` itself; and asking again with the same answer does not write the record.
#[test]
fn a_refresh_keeps_what_github_says_on_the_record_and_writes_only_when_it_changes() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "draft", "pr", &format!("{PULL}/9"));
    let path = stub_gh_answering(
        &fixture,
        &pr_json(
            "OPEN",
            true,
            "REVIEW_REQUIRED",
            &[("COMPLETED", "SUCCESS"), ("COMPLETED", "FAILURE")],
        ),
    );
    let refresh = || {
        let out = fixture
            .command(["task", "refresh", "--json"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };

    let result = refresh();
    assert_eq!(ids(&result["open"]), [id.as_str()], "{result}");
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(
        shown["prStatus"],
        serde_json::json!({
            "state": "draft",
            "title": "A pull request",
            "review": "required",
            "ci": { "pass": 1, "fail": 1, "pending": 0 },
        }),
        "{shown}"
    );
    assert_eq!(status_of(&fixture, &id), "pr");

    let file = record_file(&fixture, &id);
    let before = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    refresh();
    let after = std::fs::metadata(&file).unwrap().modified().unwrap();
    assert_eq!(
        before, after,
        "the record was rewritten with the same answer"
    );
}

/// A merged PR's summary is stored in the same write that moves the record to done.
#[test]
fn a_merged_pr_is_kept_as_merged_when_its_record_moves_to_done() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "merged", "pr", &format!("{PULL}/1"));
    let (path, _) = stub_gh(&fixture);
    let out = fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["status"], "done");
    assert_eq!(shown["prStatus"]["state"], "merged", "{shown}");
}

/// An answer longer than a pipe holds. `gh` blocks writing it until
/// somebody reads, so a refresh that waited for it to exit first would time out and never
/// move the merged PR's record.
#[test]
fn a_pr_with_hundreds_of_checks_is_still_read() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "big", "pr", &format!("{PULL}/1"));
    let checks: Vec<(&str, &str)> = (0..2000)
        .map(|i| {
            if i % 100 == 0 {
                ("COMPLETED", "FAILURE")
            } else {
                ("COMPLETED", "SUCCESS")
            }
        })
        .collect();
    // Counted by state the way the query asks, the checks themselves are a few bytes; the
    // rest of the weight is a field the query never asked for, so the answer still fills a pipe.
    let mut answer = pr_json("MERGED", false, "APPROVED", &checks);
    answer.insert_str(
        answer.len() - 1,
        &format!(r#","padding":"{}""#, "x".repeat(200_000)),
    );
    assert!(answer.len() > 100_000, "{}", answer.len());
    let path = stub_gh_answering(&fixture, &answer);
    let out = fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let shown = fixture.json(&["task", "show", "--id", &id]);
    assert_eq!(shown["status"], "done", "{shown}");
    assert_eq!(shown["prStatus"]["ci"]["fail"], 20, "{shown}");
    assert_eq!(shown["prStatus"]["ci"]["pass"], 1980, "{shown}");
}

/// What was read about one PR does not describe the next one.
#[test]
fn pointing_a_record_at_another_pr_forgets_what_was_read_about_the_old_one() {
    let fixture = Fixture::new(QUIET);
    let id = task_with_pr(&fixture, "moved", "pr", &format!("{PULL}/9"));
    let path = stub_gh_answering(&fixture, &pr_json("OPEN", true, "", &[]));
    fixture
        .command(["task", "refresh"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_eq!(
        fixture.json(&["task", "show", "--id", &id])["prStatus"]["state"],
        "draft"
    );
    // The same PR again keeps it; another one drops it.
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--pr",
        &format!("{PULL}/9"),
        "--no-hand-over",
    ]);
    assert!(fixture.json(&["task", "show", "--id", &id])["prStatus"].is_object());
    fixture.ok(&[
        "task",
        "update",
        "--id",
        &id,
        "--pr",
        &format!("{PULL}/10"),
        "--no-hand-over",
    ]);
    assert!(fixture.json(&["task", "show", "--id", &id])["prStatus"].is_null());
}

/// A bare number is read on the host the repository's origin is on, not assumed to be
/// github.com's.
#[test]
fn a_bare_pr_number_is_read_on_the_host_of_the_origin() {
    let fixture = Fixture::new(QUIET);
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .hermetic()
            .args(args)
            .current_dir(&fixture.repo)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    };
    git(&[
        "remote",
        "set-url",
        "origin",
        "git@git.example.com:acme/widget.git",
    ]);
    let id = task_with_pr(&fixture, "bare", "pr", "2");
    let (path, _) = stub_gh(&fixture);
    // The stub's own log knows the numbers; what it was told about the host is in its args.
    let gh = fixture.repo.join("stub-bin").join("gh");
    let script = std::fs::read_to_string(&gh).unwrap().replace(
        "echo call >>",
        "echo \"$*\" | sed -n 's/.*--hostname \\([^ ]*\\).*/\\1/p' >> \"$ADJUTANT_STATE_DIR/gh-hosts\"\necho call >>",
    );
    std::fs::write(&gh, script).unwrap();
    let out = fixture
        .command(["task", "refresh", "--json"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert!(out.status.success());
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(ids(&result["open"]), [id.as_str()], "{result}");
    let hosts = std::fs::read_to_string(fixture.state.join("gh-hosts")).unwrap();
    assert_eq!(hosts.trim(), "git.example.com");
}
