//! Closing the tab a worker sits in, and when the record may be cleared.

mod common;

use common::*;

/// A config whose close template is `command`, for this fixture's repository.
///
/// Assembled with `json!` rather than pasted into a literal: these templates carry paths,
/// and a path with a quote or a backslash in it turns a hand-escaped config file into a
/// test that fails for a reason it is not about.
fn closing_with(close: impl Into<serde_json::Value>) -> String {
    serde_json::json!({
        "notification": "true",
        "terminal": { "close": close.into() },
        "repos": { "acme/widget": {
            "taskSource": "github", "issueRepo": "acme/widget",
            "issueKeys": { "acme/widget": "WID" }, "ide": "code"
        }}
    })
    .to_string()
}

/// A worker record for `pid`, carrying the start time the system reports for it — the
/// anchor that tells that process from whatever the pid is handed to next.
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

/// A process standing in for a worker: it stays until something kills it, and really does
/// disappear when something does.
///
/// Not simply a `sleep` spawned by the test. That would be the test's own child, and a
/// child nobody has waited for stays a zombie once it dies — `ps -o lstart=` answers for a
/// zombie exactly as it answers for a live process, so a test that killed one would be
/// asserting that `close` sees a death at the one moment the system still reports life, and
/// would pass against an implementation that never checked at all.
///
/// So it is a *grandchild*: spawned under a shell that then sits in `wait`, which gives it
/// a live parent to reap it the instant it goes. `ps` says "no such process" from then on.
///
/// It blocks on a pipe this test holds rather than sleeping for a while, so nothing here
/// depends on a stand-in outliving the rest of the test — a timeout would be a second way
/// for a correct implementation to fail, on a slow enough machine. Closing the pipe is also
/// what cleans up after a test run that was killed outright: the read reaches end of file
/// and the process exits on its own. `cat <&3` rather than plain `cat` because a shell
/// points a background job's stdin at `/dev/null` unless the redirect is written out, and
/// `cat` on `/dev/null` is a stand-in that exits immediately.
struct Sleeper {
    shell: std::process::Child,
    /// `None` only while `new` is still assembling one. The value exists before the pid is
    /// read so that a panic during construction still runs `Drop`: a value that never
    /// finished being built is never dropped, and both processes would be left behind.
    pid: Option<u32>,
}

impl Sleeper {
    fn new() -> Sleeper {
        let shell = Command::new("sh")
            .args(["-c", "exec 3<&0; cat <&3 >/dev/null & echo $!; wait"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The shell announces the kill on stderr, which is this test's own doing and
            // not something a reader of the suite's output should have to explain.
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut sleeper = Sleeper { shell, pid: None };
        let mut line = String::new();
        std::io::BufReader::new(sleeper.shell.stdout.as_mut().unwrap())
            .read_line(&mut line)
            .unwrap();
        sleeper.pid = Some(line.trim().parse().unwrap());
        assert!(
            !ps_started(sleeper.pid()).is_empty(),
            "the stand-in worker was not running to begin with"
        );
        sleeper
    }

    fn pid(&self) -> u32 {
        self.pid.expect("the stand-in worker has no pid")
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        // By closing the pipe, never by pid. Two tests kill the stand-in through the close
        // template, and the shell reaps it at once, so its pid is back with the system by the
        // time this runs — with the suite running in parallel, a second `kill` could reach
        // whatever holds that number now. Closing the pipe needs no pid: `cat` reads end of
        // file and exits, the shell reaps it and leaves `wait`, and waiting on the shell
        // then returns. A stand-in that is already gone changes nothing.
        drop(self.shell.stdin.take());
        let _ = self.shell.wait();
    }
}

#[test]
fn closing_a_worktree_nobody_is_working_in_succeeds_and_says_so() {
    // The hub calls this on its way to `git worktree remove`, so arriving twice — or
    // arriving after the worker stopped on its own — has to be a success. Made an error, a
    // cleanup that is already half done can never be finished.
    let fixture = Fixture::new(QUIET);
    let never_existed = fixture.repo.join("worktrees").join("wid-1");
    let out = fixture.ok(&["close", "--worktree", never_existed.to_str().unwrap()]);
    assert!(out.contains("no worker is running"), "{out}");

    // A record whose process is long gone gets the same answer, and the record goes with
    // it: left there, the next reader is told a worker is present in a worktree that has
    // none.
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(
        &record,
        serde_json::json!({"pid": std::process::id(), "title": "WID-957",
                           "psStarted": "Thu Jan  1 00:00:00 1970"})
        .to_string(),
    )
    .unwrap();
    let stale = fixture.ok(&["close", "--worktree", &worktree]);
    assert!(stale.contains("no worker is running"), "{stale}");
    assert!(!record.exists(), "the record left behind survived");

    assert_eq!(
        fixture.ok(&["close", "--worktree", &worktree, "--quiet"]),
        ""
    );

    // A record that names nobody is *not* the same as no record. It is a file somebody
    // wrote, in a worktree somebody may be working in, that this cannot read — and reading
    // it as "free" is the fail-open this command exists to avoid.
    for content in ["{ not json", "{}", r#"{"pid": null}"#] {
        std::fs::write(&record, content).unwrap();
        let out = fixture.cmd(&["close", "--worktree", &worktree]);
        let said = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !out.status.success(),
            "{content} was read as a free worktree"
        );
        assert!(said.contains("cannot be read as naming a worker"), "{said}");
        assert!(record.exists(), "{content} was cleared away");
    }
}

#[test]
fn closing_a_running_worker_shows_the_command_first_and_takes_the_record_with_it() {
    // The close template stands in for whatever disposes of a tab, the way the wake tests
    // stand in for whatever pokes one.
    let fixture = Fixture::new(&closing_with("true --pid {pid} --tty {tty}"));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());

    // A dry run shows the command and runs nothing, and still answers about the worktree:
    // the procedures chain this into `&& git worktree remove`.
    let planned = fixture.cmd(&["close", "--worktree", &worktree, "--dry-run"]);
    let planned_out = String::from_utf8_lossy(&planned.stdout).to_string();
    assert!(
        planned_out.contains(&format!("--pid {}", worker.pid())),
        "{planned_out}"
    );
    assert!(
        !planned.status.success(),
        "a dry run said the worktree is free"
    );
    assert!(record.exists(), "a dry run cleared the record");

    // A close that failed is not a closed tab. The caller is on its way to `git worktree
    // remove`, so this has to be a non-zero exit and the record has to stay.
    std::fs::write(&fixture.config, closing_with("false")).unwrap();
    let refused = fixture.cmd(&["close", "--worktree", &worktree]);
    assert!(!refused.status.success());
    assert!(record.exists(), "a tab that never closed lost its record");
    // The premise of the phase below, and of the two above it: nothing so far was supposed
    // to touch the worker, and a stand-in that had already died would make the rest of this
    // test pass for the wrong reason.
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the stand-in worker died before the close that is meant to kill it"
    );

    // And the whole way through. The template kills the process the way closing its tab
    // would, and only then is the worktree reported free and the record cleared.
    std::fs::write(&fixture.config, closing_with("kill {pid}")).unwrap();
    let done = fixture.ok(&["close", "--worktree", &worktree]);
    assert!(done.contains("closed the tab"), "{done}");
    assert!(!record.exists(), "the record outlived the tab it named");
    assert!(
        ps_started(worker.pid()).is_empty(),
        "close reported a death the system disagrees with"
    );
}

#[test]
fn a_close_that_leaves_the_worker_running_clears_nothing_and_says_so() {
    // What the close command reports is not what the caller needs to know: a cancelled
    // confirmation dialog and a template resolved to the wrong pane both report a close
    // having disposed of nothing. `true` is exactly that command.
    let fixture = Fixture::new(&closing_with("true"));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "a live worker was reported as cleared away: {said}"
    );
    assert!(
        said.contains(&format!("pid {} is still there", worker.pid())),
        "{said}"
    );
    assert!(record.exists(), "the record of a live worker was cleared");
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the stand-in worker died"
    );
}

#[test]
fn a_record_that_vanished_is_not_evidence_the_worker_died() {
    // The record is a note about a process, not the process. This close command removes the
    // note and leaves the worker alone, which is what a template pointed at the wrong thing
    // does from here. Ask the *record* whether the worker went, and it answers "gone".
    let fixture = Fixture::new(QUIET);
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());
    std::fs::write(
        &fixture.config,
        closing_with(format!("rm -f {}", shell_quoted(&record.to_string_lossy()))),
    )
    .unwrap();

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "a worktree whose record was deleted was called free: {said}"
    );
    assert!(
        said.contains(&format!("pid {} is still there", worker.pid())),
        "{said}"
    );
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the stand-in worker died"
    );
    // The premise. Without it, a close template that quietly stopped removing the record
    // would leave this test indistinguishable from the one above it: still passing, and no
    // longer about anything.
    assert!(
        !record.exists(),
        "the close command did not remove the record this test is about"
    );
}

#[test]
fn a_record_with_no_start_time_is_not_acted_on() {
    // `register_worker` writes `psStarted: null` when `ps` would not answer at that moment.
    // Without it there is nothing to tell this worker from the next process to be handed
    // that pid — and a tab is closed on the answer, so "the number is in use, close it" is
    // not good enough.
    let fixture = Fixture::new(&closing_with("true"));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = fixture.repo.join(".claude").join("adjutant-worker.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    for named in [
        serde_json::json!({"pid": worker.pid(), "title": "WID-957", "psStarted": null}),
        serde_json::json!({"pid": worker.pid(), "title": "WID-957"}),
    ] {
        std::fs::write(&record, named.to_string()).unwrap();
        let out = fixture.cmd(&["close", "--worktree", &worktree]);
        let said = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(!out.status.success(), "{named} was acted on: {said}");
        assert!(
            said.contains(&format!("cannot tell whether pid {}", worker.pid())),
            "{said}"
        );
        assert!(record.exists(), "{named} was cleared away");
    }
}

#[test]
fn closing_can_be_turned_off_and_then_nothing_is_cleared() {
    // The config's promise for every one of these keys is that `false` turns the behaviour
    // off, which is a different answer from leaving it out. Read as unset, `"close": false`
    // would reach the built-in closer and dispose of the tab it was meant to protect.
    //
    // And with nothing closing tabs, nothing may report a worktree as finished with: the
    // person who turned this off is doing the closing by hand.
    let fixture = Fixture::new(&closing_with(false));
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "a worktree nobody closed was called free: {said}"
    );
    assert!(said.contains("turned off"), "{said}");
    assert!(
        record.exists(),
        "the record was cleared without a tab being closed"
    );
    assert!(
        !ps_started(worker.pid()).is_empty(),
        "the worker was closed anyway"
    );
}

#[test]
fn a_record_naming_another_worker_is_left_where_it_is() {
    // The window between seeing a worker go and clearing its record: a new worker registers
    // in the same worktree in between. Clearing the record then reports a free worktree
    // about somebody who has only just started, and the next call agrees with it, because
    // by then there is no record at all.
    //
    // The template does both halves in the order that hurts: it kills the worker being
    // closed and registers the newcomer before `close` gets to look again.
    let fixture = Fixture::new(QUIET);
    let worker = Sleeper::new();
    let worktree = fixture.repo.to_str().unwrap().to_string();
    let record = forge_worker_record(&fixture.repo, worker.pid());
    let newcomer = serde_json::json!({
        "pid": std::process::id(), "title": "WID-958",
        "psStarted": ps_started(std::process::id()),
    })
    .to_string();
    std::fs::write(
        &fixture.config,
        closing_with(format!(
            "kill {{pid}} && printf %s {} > {}",
            shell_quoted(&newcomer),
            shell_quoted(&record.to_string_lossy())
        )),
    )
    .unwrap();

    let out = fixture.cmd(&["close", "--worktree", &worktree]);
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !out.status.success(),
        "the newcomer's worktree was reported free: {said}"
    );
    assert!(said.contains("another worker has registered"), "{said}");
    // Both halves of the premise: the worker being closed really went, and what was left
    // behind really is the newcomer's record.
    assert!(
        ps_started(worker.pid()).is_empty(),
        "the worker this test kills is still running"
    );
    let left = std::fs::read_to_string(&record).unwrap();
    assert!(
        left.contains(&std::process::id().to_string()),
        "the newcomer's record was cleared: {left}"
    );
}
