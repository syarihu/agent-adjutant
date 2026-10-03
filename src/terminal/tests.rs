use super::*;
use crate::config::Hook;

#[test]
fn a_tmux_socket_is_the_same_path_however_it_is_spelled() {
    let path = |socket, env, tmpdir| tmux_socket_path(socket, env, tmpdir, 501);
    let default = std::path::PathBuf::from("/tmp/tmux-501/default");
    assert_eq!(path(None, None, None), default);
    assert_eq!(path(Some("default"), None, None), default);
    assert_eq!(path(Some("  "), None, None), default);
    assert_eq!(
        path(Some("work"), None, Some("/var/run")),
        std::path::PathBuf::from("/var/run/tmux-501/work")
    );
    assert_ne!(
        path(Some("work"), None, None),
        path(Some("play"), None, None)
    );
    // An empty `TMUX_TMPDIR` is as good as none, as it is to tmux.
    assert_eq!(path(None, None, Some("")), default);
    assert_eq!(
        path(Some("/x/own.sock"), Some("/y/z,1,0"), None),
        std::path::PathBuf::from("/x/own.sock")
    );
    // Inside tmux, no socket means the server the caller is in.
    assert_eq!(
        path(None, Some("/x/y,123,0"), None),
        std::path::PathBuf::from("/x/y")
    );
    assert_eq!(path(None, Some(""), None), default);
    assert_eq!(path(None, Some(",1,0"), None), default);
    // A name still means the directory's file, in tmux or not.
    assert_eq!(
        path(Some("work"), Some("/x/y,123,0"), None),
        std::path::PathBuf::from("/tmp/tmux-501/work")
    );
}

/// A line the terminal would mangle is put in a file instead. The failure this prevents
/// is silent: the tab opens and runs something that was never written.
#[test]
fn an_overlong_command_is_staged_in_a_file() {
    let long = format!("echo {}", "x".repeat(MAX_INLINE_COMMAND));
    let staged = stage_command(&long).unwrap();
    let path = staged.strip_prefix("sh ").unwrap().trim_matches('\'');
    let written = std::fs::read_to_string(path).unwrap();
    assert!(written.starts_with("#!/bin/sh\n"), "{written}");
    assert!(written.contains(&long), "{written}");
    assert!(staged.len() < MAX_INLINE_COMMAND, "{staged}");
    let _ = std::fs::remove_file(path);
}

/// A dry run is read by a person. Handing them `sh /tmp/…` would hide the one thing
/// they asked to see.
#[test]
fn a_dry_run_shows_the_command_rather_than_a_path_to_it() {
    let long = format!("claude {}", "y".repeat(MAX_INLINE_COMMAND));
    let term = TerminalSettings {
        spawn: Some("tmux new-window -c {cwd} -n {title} {command}".into()),
        ..Default::default()
    };
    let done = spawn(
        &term,
        &SpawnRequest {
            cwd: ".",
            title: "t",
            command: &long,
            title_command: None,
        },
        true,
    )
    .unwrap();
    assert!(done.script.contains(&long), "{}", done.script);
    assert!(!done.script.contains("adjutant-spawn-"), "{}", done.script);
}

/// A short one is left alone, so the common case stays inspectable and leaves no files.
#[test]
fn a_short_command_is_not_staged() {
    let term = TerminalSettings {
        spawn: Some("tmux new-window -c {cwd} -n {title} {command}".into()),
        ..Default::default()
    };
    let done = spawn(
        &term,
        &SpawnRequest {
            cwd: ".",
            title: "t",
            command: "claude --help",
            title_command: None,
        },
        false,
    );
    // Running tmux may fail on this machine; what matters is what was going to run.
    let script = match done {
        Ok(done) => done.script,
        Err(e) => e,
    };
    assert!(!script.contains("adjutant-spawn-"), "{script}");
}

#[test]
fn a_title_with_a_newline_cannot_submit_a_line_in_the_new_tab() {
    let title = sanitise_title("WID-957\nrm -rf /", "fallback".into());
    assert_eq!(title, "WID-957 rm -rf /");
    // One newline inside the AppleScript string literal would run `rm -rf /` in the new
    // tab, so the check is on the generated script, not just on the title.
    let script = iterm_spawn_script(&format!(
        "cd /tmp && tmux rename-window {}",
        sh_quote(&title)
    ));
    assert_eq!(script.lines().filter(|l| l.contains("rm -rf /")).count(), 1);
    assert!(script.contains("'WID-957 rm -rf /'"), "{script}");
}

#[test]
fn a_quote_in_a_title_is_escaped_rather_than_dropped() {
    assert_eq!(applescript_literal(r#"it"s \ here"#), r#"it\"s \\ here"#);
}

#[test]
fn a_long_title_is_truncated_by_display_width_not_byte_count() {
    let title = sanitise_title(&"あ".repeat(40), "fallback".into());
    assert!(display_width(&title) <= 30, "{title}");
    assert!(title.ends_with('…'));
}

#[test]
fn a_short_title_is_left_exactly_as_it_is() {
    assert_eq!(
        sanitise_title("WID-957 検索が潰れる", "x".into()),
        "WID-957 検索が潰れる"
    );
}

#[test]
fn an_empty_title_falls_back_to_the_directory_name() {
    assert_eq!(sanitise_title("   ", "widget".into()), "widget");
}

#[test]
fn the_builtin_spawn_cds_first_and_keeps_the_command_in_one_piece() {
    let out = spawn(
        &TerminalSettings::default(),
        &SpawnRequest {
            cwd: "/tmp",
            title: "WID-957",
            command: "claude 'go now'",
            title_command: None,
        },
        true,
    )
    .unwrap();
    assert!(!out.ran);
    assert!(
        out.script.contains("cd /tmp && claude 'go now'"),
        "{}",
        out.script
    );
    assert!(out.script.contains("create tab with default profile"));
    // Naming happens in the tab, not through the terminal's API.
    assert!(!out.script.contains("set name to"), "{}", out.script);
}

#[test]
fn a_terminal_template_receives_the_whole_command_as_one_shell_line() {
    let term = TerminalSettings {
        spawn: Some("tmux new-window -c {cwd} -n {title} {command}".into()),
        ..Default::default()
    };
    let out = spawn(
        &term,
        &SpawnRequest {
            cwd: "/tmp",
            title: "WID-957",
            command: "claude 'go now'",
            title_command: None,
        },
        true,
    )
    .unwrap();
    // No `cd` in front: the template said `-c {cwd}`, and doubling up would hand tmux
    // `cd` as the command to run.
    assert_eq!(
        out.script,
        "tmux new-window -c /tmp -n WID-957 claude 'go now'"
    );
}

#[test]
fn every_documented_spawn_template_produces_a_valid_command() {
    // Each of these puts {command} in an argv slot: the three the shipped example
    // config offers, plus two a person would plausibly write. Prepending `cd … &&`
    // or `( … ) &&` there is a syntax error, so this asserts on a real shell's
    // verdict rather than on the string's shape.
    for template in [
        "tmux new-window -d -c {cwd} -n {title} {command}",
        "ghostty --working-directory={cwd} -e {command}",
        "wezterm cli spawn --cwd {cwd} -- {command}",
        "sh -c '{command}'",
        "open -a Terminal {command}",
    ] {
        let term = TerminalSettings {
            spawn: Some(template.into()),
            ..Default::default()
        };
        let done = spawn(
            &term,
            &SpawnRequest {
                cwd: "/tmp",
                title: "WID-1",
                command: "echo hi",
                title_command: Some("name-it --title WID-1"),
            },
            true,
        )
        .unwrap();
        let checked = std::process::Command::new("sh")
            .args(["-n", "-c", &done.script])
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{template} produced invalid shell: {}\n{}",
            done.script,
            String::from_utf8_lossy(&checked.stderr)
        );
    }
}

#[test]
fn a_template_taking_an_argv_gets_one_plain_command() {
    // `{cwd}` says the terminal handles the directory, so it is putting {command} in an
    // argv slot: no `cd`, and no tab naming either — that terminal names its own tabs
    // through `{title}`.
    let term = TerminalSettings {
        spawn: Some("tmux new-window -c {cwd} -n {title} {command}".into()),
        ..Default::default()
    };
    let done = spawn(
        &term,
        &SpawnRequest {
            cwd: "/tmp",
            title: "WID-1",
            command: "echo hi",
            title_command: Some("name-it --title WID-1"),
        },
        true,
    )
    .unwrap();
    assert_eq!(done.script, "tmux new-window -c /tmp -n WID-1 echo hi");
}

#[test]
fn a_terminal_template_with_no_cwd_placeholder_gets_a_cd_instead() {
    let term = TerminalSettings {
        spawn: Some("open -a Terminal {command}".into()),
        ..Default::default()
    };
    let out = spawn(
        &term,
        &SpawnRequest {
            cwd: "/tmp",
            title: "WID-957",
            command: "claude 'go now'",
            title_command: Some("name-it"),
        },
        true,
    )
    .unwrap();
    assert_eq!(
        out.script,
        "open -a Terminal cd /tmp && ( name-it || true ) && claude 'go now'"
    );
}

#[test]
fn spawning_into_a_directory_that_is_not_there_fails_before_opening_anything() {
    let err = spawn(
        &TerminalSettings::default(),
        &SpawnRequest {
            cwd: "/definitely/not/here",
            title: "x",
            command: "true",
            title_command: None,
        },
        true,
    )
    .unwrap_err();
    assert!(err.contains("no such directory"), "{err}");
}

#[test]
fn focus_without_a_template_on_a_pid_with_no_terminal_is_a_quiet_no_op() {
    // pid 1 has no controlling terminal on macOS or Linux.
    let out = focus(&TerminalSettings::default(), 1, "hub", true).unwrap();
    assert!(!out.ran);
}

#[test]
fn a_process_with_no_terminal_is_recognised_on_either_system() {
    // The spelling is the system's, not the process's: macOS prints `??` where Linux
    // prints `?`, and whichever machine this is running on can only show one of them.
    assert_eq!(tty_in("??\n"), None);
    assert_eq!(tty_in("?\n"), None);
    assert_eq!(tty_in(""), None);
    assert_eq!(tty_in("ttys004\n"), Some("ttys004".into()));
    assert_eq!(tty_in(" pts/3 \n"), Some("pts/3".into()));
}

#[test]
fn closing_without_a_template_on_a_pid_with_no_terminal_is_a_quiet_no_op() {
    // Same shape as `focus`: pid 1 has no controlling terminal, and there is no tab to
    // dispose of for a session nobody can locate.
    let out = close(&TerminalSettings::default(), 1, "WID-957", true).unwrap();
    assert!(!out.ran);
    assert!(out.description.contains("not closing"), "{out:?}");
    assert!(out.script.is_empty(), "{out:?}");
}

#[test]
fn closing_can_be_turned_off_and_then_reaches_nothing() {
    // `false` is a different answer from an absent key, and the difference is the whole
    // point here: read as unset, the built-in would dispose of the very tab somebody
    // had just declared off limits. `ran` has to stay false too, or the caller reads
    // "turned off" as "the tab is gone" and carries on removing the worktree.
    let term = TerminalSettings {
        close: Hook::Off,
        ..Default::default()
    };
    let done = close_with(
        |_| panic!("a close that is turned off ran a command"),
        Some("ttys004".to_string()),
        &term,
        std::process::id(),
        "WID-957",
        false,
    )
    .unwrap();
    assert!(!done.ran, "{done:?}");
    assert!(done.script.is_empty(), "{done:?}");
    assert!(done.description.contains("turned off"), "{done:?}");
}

#[test]
fn a_close_template_is_handed_the_pid_and_the_tty_and_runs_only_for_real() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("closed");
    // Quoted, because a `TMPDIR` with a space in it is the machine's business and not a
    // defect in what is under test — unquoted, this test failed on a correct `close`.
    let template = format!(
        "printf '%s|%s' {{pid}} {{tty}} > {}",
        sh_quote(&marker.to_string_lossy())
    );

    let term = TerminalSettings {
        close: Hook::Command(template),
        ..Default::default()
    };
    let planned = close(&term, 4321, "WID-957", true).unwrap();
    assert!(!planned.ran);
    assert!(planned.script.contains("4321"), "{}", planned.script);
    // A pid with no terminal still substitutes, as the empty string. Left standing, the
    // literal `{tty}` would be handed to a shell as an argument.
    assert!(!planned.script.contains("{tty}"), "{}", planned.script);
    assert!(!marker.exists(), "a dry run ran the template");

    let ours = std::process::id();
    let done = close(&term, ours, "WID-957", false).unwrap();
    assert!(done.ran);
    let recorded = std::fs::read_to_string(&marker).unwrap();
    let (pid, tty) = recorded.split_once('|').unwrap();
    assert_eq!(pid, ours.to_string());
    // Whatever this test is running under — a terminal or a pipe with no tty at all —
    // the template is given the answer for the pid it was asked about.
    assert_eq!(tty, tty_of(ours).unwrap_or_default());
}

/// The finding, one command over from where it was found the first time: the built-in
/// walks every iTerm2 window and, having matched no tty, runs to the end and exits 0 —
/// which is what a worker in any other terminal, in tmux, or over ssh looks like. So
/// `ran` has to mean "the command reported reaching a session", and a walk that matched
/// nothing has to be `false`.
///
/// Driven through `close_with` rather than through the helper, so the branch that reads
/// the answer cannot be deleted with this test still passing.
#[test]
fn the_builtin_close_only_reports_success_when_it_reached_a_session() {
    let ours = std::process::id();
    let term = TerminalSettings::default();

    let quiet = close_with(
        |_| Ok(String::new()),
        Some("ttys004".to_string()),
        &term,
        ours,
        "WID-957",
        false,
    )
    .unwrap();
    assert!(!quiet.ran, "{quiet:?}");
    assert!(quiet.description.contains("nothing closed"), "{quiet:?}");

    // The same command, having closed a session, says so.
    let done = close_with(
        |_| Ok(CLOSED_MARKER.to_string()),
        Some("ttys004".to_string()),
        &term,
        ours,
        "WID-957",
        false,
    )
    .unwrap();
    assert!(done.ran, "{done:?}");

    // A configured template is answered for by its exit status alone.
    let template_term = TerminalSettings {
        close: Hook::Command("close-tab --pid {pid}".to_string()),
        ..Default::default()
    };
    let template = close_with(
        |_| Ok(String::new()),
        None,
        &template_term,
        ours,
        "WID-957",
        false,
    )
    .unwrap();
    assert!(template.ran, "{template:?}");

    // A command that failed is not a closed tab, and unlike `wake` it is not swallowed.
    let failed = close_with(
        |_| Err("no iTerm2 window is open".to_string()),
        Some("ttys004".to_string()),
        &term,
        ours,
        "WID-957",
        false,
    );
    assert!(failed.is_err(), "{failed:?}");

    // The contract the two halves share, checked where it can actually break: the
    // marker has to sit *inside* the branch that matched the tty. Hoisted out of that
    // branch it would be printed by a walk that closed nothing — which is the defect
    // itself — and an assertion that only knew the marker came after `close s` would
    // have passed anyway.
    let script = iterm_close_script("ttys004");
    let matched = script.find("if tty of s is \"/dev/ttys004\" then").unwrap();
    let closes = script.find("close s").unwrap();
    let marked = script.find(CLOSED_MARKER).unwrap();
    let branch_ends = script.find("end if").unwrap();
    assert!(matched < closes, "{script}");
    assert!(closes < marked, "{script}");
    assert!(marked < branch_ends, "{script}");
    // And the walk that matched nothing has to fall out saying nothing.
    assert!(script.trim_end().ends_with("return \"\""), "{script}");
}

#[test]
fn naming_this_tab_is_a_template_like_everything_else() {
    let term = TerminalSettings {
        title: Hook::Command("tmux rename-window {title}".into()),
        ..Default::default()
    };
    let done = set_title(&term, "🗂 hub widget", true).unwrap();
    assert_eq!(done.script, "tmux rename-window '🗂 hub widget'");
}

#[test]
fn the_builtin_title_writes_to_a_terminal_rather_than_to_stdout() {
    // stdout is whatever captured the process — for an agent's shell tool, the
    // transcript. The escape sequence has to reach the tty or it is just noise.
    let done = set_title(&TerminalSettings::default(), "hub", true).unwrap();
    if !done.script.is_empty() {
        assert!(done.script.contains("> /dev/"), "{}", done.script);
        assert!(done.script.contains("033]0;"), "{}", done.script);
    }
}

#[test]
fn a_title_that_is_turned_off_runs_nothing_at_all() {
    let term = TerminalSettings {
        title: Hook::Off,
        ..Default::default()
    };
    let done = set_title(&term, "hub", false).unwrap();
    assert!(!done.ran);
    assert!(done.script.is_empty());
}

#[test]
fn waking_a_hub_is_a_template_and_can_be_turned_off() {
    let done = wake(
        &TerminalSettings::default(),
        &Wake {
            hook: Hook::Command("tmux send-keys -t {tty} {line} Enter".into()),
            line: None,
        },
        4321,
        "画像が潰れる",
        HUB_WAKE_LINE,
        Agent::Generic,
        true,
    )
    .unwrap();
    assert!(
        done.script.starts_with("tmux send-keys -t "),
        "{}",
        done.script
    );
    assert!(done.script.contains(HUB_WAKE_LINE), "{}", done.script);

    let off = wake(
        &TerminalSettings::default(),
        &Wake {
            hook: Hook::Off,
            line: None,
        },
        4321,
        "s",
        HUB_WAKE_LINE,
        Agent::Generic,
        false,
    )
    .unwrap();
    assert!(!off.ran);
    assert!(off.script.is_empty());
}

#[test]
fn each_direction_is_pointed_at_its_own_box() {
    // The message is already in the box. Typing it into the prompt too would put the
    // same text in two places, and only one of them gets acked — so the line is an
    // instruction, and it has to name the box that direction actually reads.
    assert!(HUB_WAKE_LINE.contains("adjutant_pending") && HUB_WAKE_LINE.contains("adj pending"));
    assert!(
        WORKER_WAKE_LINE.contains("adjutant_outbox") && WORKER_WAKE_LINE.contains("adj outbox")
    );
    assert_ne!(HUB_WAKE_LINE, WORKER_WAKE_LINE);
}

#[test]
fn the_sentence_can_be_replaced_without_restating_how_to_poke() {
    // The default names MCP tools. An agent that only has the CLI, or one that wants a
    // slash command, needs a different sentence through the same terminal.
    let done = wake(
        &TerminalSettings::default(),
        &Wake {
            hook: Hook::Command("tmux send-keys -t {tty} {line} Enter".into()),
            line: Some("check adjutant pending".into()),
        },
        4321,
        "s",
        HUB_WAKE_LINE,
        Agent::Generic,
        true,
    )
    .unwrap();
    assert!(
        done.script.contains("check adjutant pending"),
        "{}",
        done.script
    );
    assert!(!done.script.contains("adjutant_pending"), "{}", done.script);
}

#[test]
fn waking_a_pid_with_no_terminal_is_a_quiet_no_op() {
    let done = wake(
        &TerminalSettings::default(),
        &Wake::default(),
        1,
        "s",
        HUB_WAKE_LINE,
        Agent::Generic,
        false,
    )
    .unwrap();
    assert!(!done.ran);
}

#[test]
fn a_focus_template_gets_the_pid() {
    let term = TerminalSettings {
        focus: Some("raise-tab --pid {pid}".into()),
        ..Default::default()
    };
    let out = focus(&term, 4321, "hub", true).unwrap();
    assert_eq!(out.script, "raise-tab --pid 4321");
}

/// The finding: a worker in any terminal other than iTerm2 was told it had been woken,
/// and because `ran` is also what suppresses the fallback notification, nobody was told
/// anything at all. Driven through `wake` itself — checking only the helper let the
/// branch that reads it be deleted with this test still passing.
#[test]
fn the_builtin_wake_only_claims_success_when_it_typed_something() {
    let built_in = Wake::default();
    let ours = std::process::id();
    let term = TerminalSettings::default();

    // The built-in, having walked every window and found no matching tab, prints
    // nothing and exits 0 — which is what a session in another terminal looks like.
    let quiet = wake_with(
        |_| Ok(String::new()),
        Some("ttys004".to_string()),
        &term,
        &built_in,
        &WakeRequest {
            pid: ours,
            subject: "s",
            line: "check your inbox",
            agent: Agent::Generic,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(!quiet.ran, "{quiet:?}");
    assert!(quiet.description.contains("nothing woken"), "{quiet:?}");

    // The same command, having typed into a session, says so.
    let woken = wake_with(
        |_| Ok(WOKE_MARKER.to_string()),
        Some("ttys004".to_string()),
        &term,
        &built_in,
        &WakeRequest {
            pid: ours,
            subject: "s",
            line: "check your inbox",
            agent: Agent::Generic,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(woken.ran, "{woken:?}");

    // A configured template is answered for by its exit status: it is someone else's
    // command and only it knows what success means there.
    let template = Wake {
        hook: crate::config::Hook::Command("true {pid}".to_string()),
        line: None,
    };
    let ran = wake_with(
        |_| Ok(String::new()),
        None,
        &term,
        &template,
        &WakeRequest {
            pid: ours,
            subject: "s",
            line: "check your inbox",
            agent: Agent::Generic,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(ran.ran, "{ran:?}");

    // The contract the two halves share: the marker is printed on the one path that
    // types into a session, and the fall-through returns nothing.
    let script = default_wake_command("ttys004", "check your inbox");
    let entered = script.find("write text \"\"\n").unwrap();
    let marked = script.find(WOKE_MARKER).unwrap();
    assert!(marked > entered, "{script}");
    assert!(script.trim_end().ends_with("return \"\"'"), "{script}");
}

/// The finding: a wake line long enough for the agent to take as a paste was typed into
/// the box with its newline and never submitted. The line goes without a newline and
/// Enter follows as a write of its own, after a pause.
#[test]
fn the_builtin_wake_presses_enter_apart_from_the_line() {
    let script = default_wake_command("ttys004", WORKER_WAKE_LINE);
    let typed = script
        .find(&format!(
            "write text \"{}\" newline NO\n",
            applescript_literal(WORKER_WAKE_LINE)
        ))
        .unwrap_or_else(|| panic!("the line is typed without its newline: {script}"));
    let paused = script.find(&format!("delay {WAKE_ENTER_DELAY}\n")).unwrap();
    let entered = script.find("write text \"\"\n").unwrap();
    assert!(typed < paused && paused < entered, "{script}");
    assert_eq!(script.matches("write text").count(), 2, "{script}");
}

#[test]
fn a_pane_line_reads_with_or_without_the_activity_column() {
    let panes = parse_tmux_panes(
        "%0\t1\t/dev/ttys001\t@0\ts\t0\tw\t1700000000\n%1\t2\t/dev/ttys002\t@1\ts\t1\tv\n",
    );
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].window_activity, Some(1_700_000_000));
    assert_eq!(panes[1].window_activity, None);
}

fn pane_of(session: &str, window: &str) -> TmuxPane {
    TmuxPane {
        pane_id: format!("%{window}"),
        pane_pid: 1,
        pane_tty: String::new(),
        window_id: window.to_string(),
        session_name: session.to_string(),
        window_index: 0,
        window_name: "w".to_string(),
        window_activity: None,
    }
}

fn client_of(session: &str, control: bool, window: &str) -> TmuxClient {
    TmuxClient {
        session: session.to_string(),
        control,
        window_id: window.to_string(),
    }
}

#[test]
fn tmux_clients_are_read_from_their_three_fields() {
    let clients = parse_tmux_clients("main\t0\t@1\nmain\t1\t@2\nshort\t0\n");
    assert_eq!(
        clients,
        vec![
            client_of("main", false, "@1"),
            client_of("main", true, "@2")
        ]
    );
}

#[test]
fn attached_counts_leave_out_the_board_and_follow_the_kind_of_client() {
    let panes = [
        pane_of("main", "@1"),
        pane_of("main", "@2"),
        pane_of("main", "@2"),
        pane_of("adjboard-7-1", "@1"),
        pane_of("other", "@3"),
    ];
    let clients = [
        client_of("adjboard-7-1", false, "@1"),
        client_of("main", false, "@1"),
        client_of("other", true, "@3"),
        client_of("main", true, "@2"),
    ];
    let counts = attached_counts(&panes, &clients);
    // A normal client is on its current window only; a control client on all of its
    // session's windows, once each however many panes they have; the board on none.
    assert_eq!(counts.get("@1"), Some(&2));
    assert_eq!(counts.get("@2"), Some(&1));
    assert_eq!(counts.get("@3"), Some(&1));
    let idle = attached_counts(&panes, &[]);
    assert_eq!(idle.get("@1"), Some(&0));
    assert_eq!(idle.get("@9"), None);
}

#[test]
fn listing_clients_of_no_server_is_nobody() {
    let none = list_tmux_clients_with(|_| Err("no server running on /tmp/x".to_string()), None);
    assert!(none.is_empty());
    let one = list_tmux_clients_with(|_| Ok("s\t0\t@1\n".to_string()), None);
    assert_eq!(one.len(), 1);
}

#[test]
fn tmux_pane_parsing_and_matching() {
    let raw = "\
%0\t1000\t/dev/ttys001\t@0\tadjutant\t0\tmain
%1\t2000\t/dev/ttys002\t@1\tadjutant\t1\tworker-task
%2\t3000\t/dev/ttys003\t@2\tother session with space\t0\tworker with spaces in title
";
    let panes = parse_tmux_panes(raw);
    assert_eq!(panes.len(), 3);
    assert_eq!(panes[0].pane_id, "%0");
    assert_eq!(panes[0].pane_pid, 1000);
    assert_eq!(panes[0].pane_tty, "/dev/ttys001");
    assert_eq!(panes[0].window_id, "@0");
    assert_eq!(panes[0].session_name, "adjutant");
    assert_eq!(panes[0].window_index, 0);
    assert_eq!(panes[0].window_name, "main");

    assert_eq!(panes[2].session_name, "other session with space");
    assert_eq!(panes[2].window_name, "worker with spaces in title");

    // Matching by tty (with and without /dev/ prefix)
    let found = find_matching_pane(&panes, None, Some("ttys002")).unwrap();
    assert_eq!(found.pane_id, "%1");
    let found2 = find_matching_pane(&panes, None, Some("/dev/ttys001")).unwrap();
    assert_eq!(found2.pane_id, "%0");

    // Matching by direct PID
    let found_pid = find_matching_pane(&panes, Some(2000), None).unwrap();
    assert_eq!(found_pid.pane_id, "%1");

    // Unknown PID / TTY
    assert!(find_matching_pane(&panes, Some(9999), Some("ttys999")).is_none());
}

#[test]
fn tmux_spawn_script_generates_session_and_window() {
    let script = tmux_spawn_script(None, "adjutant", "/tmp", "task-1", "claude --help");
    assert!(script.starts_with("tmux has-session -t '=adjutant' "));
    assert!(script.contains("new-session -d -s adjutant -n main"));
    assert!(script.contains("new-window -d -t '=adjutant:' -c /tmp -n task-1 'claude --help'"));

    let socket_script = tmux_spawn_script(Some("custom-sock"), "sess", "/dir", "title", "echo hi");
    assert!(socket_script.starts_with("tmux -L custom-sock has-session"));
    assert!(socket_script.contains("tmux -L custom-sock new-session"));
    assert!(socket_script.contains("tmux -L custom-sock new-window"));

    let path_socket_script =
        tmux_spawn_script(Some("/path/to/sock"), "sess", "/dir", "title", "echo hi");
    assert!(path_socket_script.starts_with("tmux -S /path/to/sock has-session"));
}

#[test]
fn tmux_wake_script_generates_literal_send_and_enter() {
    let script = tmux_wake_script(Some("test-sock"), "%2", "check inbox");
    assert!(script.contains("tmux -L test-sock send-keys -l -t %2 'check inbox'"));
    assert!(script.contains(&format!("sleep {WAKE_ENTER_DELAY}")));
    assert!(script.contains("tmux -L test-sock send-keys -t %2 Enter"));
    assert!(script.contains(&format!("echo {WOKE_MARKER}")));
}

#[test]
fn tmux_close_script_generates_kill_window_and_marker() {
    let script = tmux_close_script(Some("test-sock"), "@1");
    assert_eq!(
        script,
        format!("tmux -L test-sock kill-window -t @1 && echo {CLOSED_MARKER}")
    );
}

#[test]
fn a_hub_is_stopped_by_its_pane_on_its_socket() {
    assert_eq!(
        tmux_kill_pane_script(Some("scratch"), "%3"),
        format!("tmux -L scratch kill-pane -t %3 && echo {CLOSED_MARKER}")
    );
    assert_eq!(
        tmux_kill_pane_script(Some("/tmp/t.sock"), "%3"),
        format!("tmux -S /tmp/t.sock kill-pane -t %3 && echo {CLOSED_MARKER}")
    );
}

#[test]
fn tmux_focus_script_generates_select_window_and_pane() {
    let script = tmux_focus_script(Some("test-sock"), "@1", Some("%2"));
    assert!(script.contains("tmux -L test-sock select-window -t @1"));
    assert!(script.contains("tmux -L test-sock select-pane -t %2"));
}

#[test]
fn tmux_wake_with_runner_mock() {
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    let wake_cfg = Wake::default();

    let raw_panes = "%1\t12345\t/dev/ttys005\t@1\tadjutant\t1\tworker\n";
    let runner = |cmd: &str| {
        if cmd.contains("list-panes") {
            Ok(raw_panes.to_string())
        } else if cmd.contains("send-keys") {
            Ok(format!("typed\n{WOKE_MARKER}\n"))
        } else {
            Err(format!("unexpected command: {cmd}"))
        }
    };

    let performed = wake_with(
        runner,
        Some("ttys005".to_string()),
        &term,
        &wake_cfg,
        &WakeRequest {
            pid: 12345,
            subject: "sub",
            line: "wake up",
            agent: Agent::Generic,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(performed.ran, "{performed:?}");
    assert!(performed.script.contains("send-keys -l -t %1 'wake up'"));
    assert!(performed.description.contains("woke the session"));
}

#[test]
fn tmux_close_with_runner_mock() {
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };

    let raw_panes = "%1\t12345\t/dev/ttys005\t@1\tadjutant\t1\tworker\n";
    let runner = |cmd: &str| {
        if cmd.contains("list-panes") {
            Ok(raw_panes.to_string())
        } else if cmd.contains("kill-window") {
            Ok(format!("killed\n{CLOSED_MARKER}\n"))
        } else {
            Err(format!("unexpected command: {cmd}"))
        }
    };

    let performed = close_with(
        runner,
        Some("ttys005".to_string()),
        &term,
        12345,
        "worker",
        false,
    )
    .unwrap();
    assert!(performed.ran, "{performed:?}");
    assert!(performed.script.contains("kill-window -t @1"));
    assert!(performed.description.contains("closed the tab"));
}

#[test]
fn the_backend_is_the_one_spawn_would_use() {
    let custom = TerminalSettings {
        spawn: Some("wezterm cli spawn --cwd {cwd} -- {command}".into()),
        ..Default::default()
    };
    assert_eq!(backend_name(&custom), "custom");
    // A template wins over the preset in `spawn`, so it does here too.
    let both = TerminalSettings {
        preset: Some("tmux".into()),
        spawn: Some("tmux new-window {command}".into()),
        ..Default::default()
    };
    assert_eq!(backend_name(&both), "custom");
    let tmux = TerminalSettings {
        preset: Some("tmux".into()),
        ..Default::default()
    };
    assert_eq!(backend_name(&tmux), "tmux");
    assert_eq!(backend_name(&TerminalSettings::default()), "iterm2");
}

#[test]
fn a_location_inside_tmux_is_read_from_the_pane_not_the_settings() {
    // Settings that name another socket and session: what is recorded is where the
    // process actually is.
    let term = TerminalSettings {
        preset: Some("tmux".into()),
        socket: Some("elsewhere".into()),
        session: Some("other".into()),
        ..Default::default()
    };
    let runner = |cmd: &str| {
        assert!(cmd.contains("tmux -u -S /tmp/tmux-501/default"), "{cmd}");
        assert!(cmd.contains("display-message -p -t %7"), "{cmd}");
        Ok("work\t@3\n".to_string())
    };
    let at = location_with(
        runner,
        Some("/tmp/tmux-501/default,4242,0"),
        Some("%7"),
        &term,
    );
    assert_eq!(
        at,
        SessionTerminal {
            backend: "tmux".into(),
            socket: Some("/tmp/tmux-501/default".into()),
            session: Some("work".into()),
            window: Some("@3".into()),
            pane: Some("%7".into()),
        }
    );

    // tmux not answering still leaves the socket and pane, which are enough to find it.
    let at = location_with(
        |_: &str| Err("no server running".to_string()),
        Some("/tmp/tmux-501/default,4242,0"),
        Some("%7"),
        &term,
    );
    assert_eq!(at.backend, "tmux");
    assert_eq!(at.pane.as_deref(), Some("%7"));
    assert_eq!(at.window, None);
}

#[test]
fn a_location_outside_tmux_carries_only_the_backend() {
    let custom = TerminalSettings {
        spawn: Some("wezterm cli spawn --cwd {cwd} -- {command}".into()),
        ..Default::default()
    };
    let at = location_with(
        |cmd: &str| Err(format!("nothing should be asked: {cmd}")),
        None,
        None,
        &custom,
    );
    assert_eq!(at.backend, "custom");
    assert_eq!(
        (at.socket, at.session, at.window, at.pane),
        (None, None, None, None)
    );
}

#[test]
fn a_location_recorded_from_a_board_session_names_the_real_one() {
    let term = TerminalSettings::default();
    let at = location_with(
        |cmd: &str| {
            assert!(cmd.contains("#{session_group}"), "{cmd}");
            Ok("adjboard-42-1\t@3\twork\n".to_string())
        },
        Some("/tmp/tmux-501/default,4242,0"),
        Some("%7"),
        &term,
    );
    assert_eq!(at.session.as_deref(), Some("work"));
    assert_eq!(at.window.as_deref(), Some("@3"));

    // A group that is not ours does not replace the session's own name.
    let at = location_with(
        |_: &str| Ok("dev\t@3\tteam\n".to_string()),
        Some("/tmp/tmux-501/default,4242,0"),
        Some("%7"),
        &term,
    );
    assert_eq!(at.session.as_deref(), Some("dev"));
}

#[test]
fn socket_arguments_follow_the_prefix_rule() {
    assert_eq!(
        tmux_socket_args(Some("/tmp/t/default")),
        ["-S", "/tmp/t/default"]
    );
    assert_eq!(tmux_socket_args(Some("adj-test")), ["-L", "adj-test"]);
    assert!(tmux_socket_args(None).is_empty());
    assert!(tmux_socket_args(Some("  ")).is_empty());
}

#[test]
fn tmux_versions_are_read_from_what_dash_v_prints() {
    assert_eq!(parse_tmux_version("tmux 3.7c\n"), Some((3, 7)));
    assert_eq!(parse_tmux_version("tmux 3.1"), Some((3, 1)));
    assert_eq!(parse_tmux_version("tmux 2.9a"), Some((2, 9)));
    assert_eq!(parse_tmux_version("tmux next-3.5"), Some((3, 5)));
    assert_eq!(parse_tmux_version("tmux master"), Some((u32::MAX, 0)));
    assert_eq!(parse_tmux_version("tmux"), None);
    assert_eq!(parse_tmux_version("zsh: command not found: tmux"), None);
}

#[test]
fn the_window_home_is_asked_by_window_id_and_names_the_group_to_join() {
    let script = tmux_window_home_script(Some("adj-test"), "@4");
    assert_eq!(
        script,
        "tmux -u -L adj-test display-message -p -t @4 '#{session_name}\t#{session_group}'"
    );
    assert_eq!(parse_window_home("work\tteam\n").as_deref(), Some("team"));
    assert_eq!(parse_window_home("work\t\n").as_deref(), Some("work"));
    assert_eq!(parse_window_home("work").as_deref(), Some("work"));
    assert_eq!(parse_window_home("\n"), None);
    // Names that are not ASCII are the answer as they are, and the default server (no
    // socket) gets the flag too.
    assert_eq!(parse_window_home("ｓ日本-a\tg1\n").as_deref(), Some("g1"));
    assert_eq!(
        parse_window_home("ｓ日本-a\t\n").as_deref(),
        Some("ｓ日本-a")
    );
    assert!(tmux_window_home_script(None, "@4").starts_with("tmux -u display-message"));
    assert!(
        tmux_window_home_script(Some("/tmp/t/sock"), "@4")
            .starts_with("tmux -u -S /tmp/t/sock display-message")
    );
}

#[test]
fn the_prepare_script_makes_a_grouped_session_and_picks_the_window_by_id() {
    let script = board_attach_prepare_script(Some("/tmp/t/sock"), "work", "adjboard-1-2", "@5");
    assert_eq!(
        script,
        "tmux -S /tmp/t/sock new-session -d -s adjboard-1-2 -t work && \
         tmux -S /tmp/t/sock select-window -t '=adjboard-1-2:@5'"
    );
    assert!(!script.contains("select-pane"), "{script}");
    assert!(!script.contains("window-size"), "{script}");
    assert!(!script.contains("ignore-size"), "{script}");
    assert!(!script.contains("destroy-unattached"), "{script}");
    // No hooks: one on a session that is destroyed later crashed the tmux server.
    assert!(!script.contains("set-hook"), "{script}");

    let spaced = board_attach_prepare_script(Some("adj-test"), "my session", "adjboard-1-2", "@5");
    assert!(
        spaced.starts_with("tmux -L adj-test new-session -d -s adjboard-1-2 -t 'my session' &&"),
        "{spaced}"
    );
}

#[test]
fn the_watch_scripts_address_only_the_board_session() {
    assert_eq!(
        board_windows_script(Some("adj-test"), "adjboard-1-2"),
        "tmux -L adj-test list-windows -t '=adjboard-1-2' -F '#{window_id}'"
    );
    assert_eq!(
        board_detach_script(Some("/tmp/t/sock"), "adjboard-1-2"),
        "tmux -S /tmp/t/sock detach-client -s '=adjboard-1-2'"
    );
}

#[test]
fn the_attach_command_is_gated_on_the_tmux_version() {
    assert_eq!(
        board_attach_args(Some("adj-test"), "adjboard-1-2", true),
        [
            "-u",
            "-L",
            "adj-test",
            "attach-session",
            "-E",
            "-f",
            "active-pane",
            "-t",
            "=adjboard-1-2"
        ]
    );
    assert_eq!(
        board_attach_args(Some("/tmp/t/sock"), "adjboard-1-2", false),
        [
            "-u",
            "-S",
            "/tmp/t/sock",
            "attach-session",
            "-E",
            "-t",
            "=adjboard-1-2"
        ]
    );
    assert_eq!(
        board_attach_args(None, "adjboard-1-2", true),
        [
            "-u",
            "attach-session",
            "-E",
            "-f",
            "active-pane",
            "-t",
            "=adjboard-1-2"
        ]
    );
    // tmux removing a session itself crashes the server, so it is never asked to.
    for active_pane in [true, false] {
        let args = board_attach_args(None, "x", active_pane);
        assert!(!args.iter().any(|a| a.contains("destroy-unattached")));
    }
}

#[test]
fn the_attach_line_is_control_mode_only_for_iterm_and_keeps_the_last_only_where_tmux_can() {
    assert_eq!(
        native_attach_line(Some("adj-test"), "adjterm-1-2", true, true),
        "tmux -u -CC -L adj-test attach-session -t '=adjterm-1-2' ';' set-option -t '=adjterm-1-2:' destroy-unattached keep-last"
    );
    assert_eq!(
        native_attach_line(Some("/tmp/t/sock"), "adjterm-1-2", false, false),
        "tmux -u -S /tmp/t/sock attach-session -t '=adjterm-1-2'"
    );
    assert_eq!(
        native_attach_line(None, "adjterm-1-2", false, true),
        "tmux -u attach-session -t '=adjterm-1-2' ';' set-option -t '=adjterm-1-2:' destroy-unattached keep-last"
    );
}

#[test]
fn the_attach_line_reaches_tmux_intact_through_zsh() {
    // zsh expands a bare word starting with `=` as a command-path lookup, so the session
    // targets must arrive quoted. `tmux` is swapped for printf to show the argv it gets.
    let line = native_attach_line(Some("adj-test"), "adjterm-1-2", true, true);
    let line = line.replacen("tmux ", "printf '%s\\n' ", 1);
    let out = match std::process::Command::new("zsh")
        .args(["-f", "-c"])
        .arg(&line)
        .output()
    {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("zsh not found; skipping");
            return;
        }
        Err(e) => panic!("{e}"),
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "-u\n-CC\n-L\nadj-test\nattach-session\n-t\n=adjterm-1-2\n;\nset-option\n-t\n=adjterm-1-2:\ndestroy-unattached\nkeep-last\n"
    );
}

#[test]
fn the_iterm_script_opens_a_window_and_carries_the_line_escaped() {
    let script = iterm_attach_script("tmux -u -CC attach-session -t \"=x\"");
    assert!(
        script.contains("create window with default profile"),
        "{script}"
    );
    assert!(script.contains("activate"), "{script}");
    assert!(
        script.contains("write text \"tmux -u -CC attach-session -t \\\"=x\\\"\""),
        "{script}"
    );
    assert!(!script.contains("create tab"), "{script}");
}

#[test]
fn a_session_made_to_open_a_terminal_counts_as_a_person_and_is_grouped_back() {
    let panes = [pane_of("main", "@1"), pane_of("adjterm-7-1", "@1")];
    let clients = [client_of("adjterm-7-1", false, "@1")];
    assert_eq!(attached_counts(&panes, &clients).get("@1"), Some(&1));
    let control = [client_of("adjterm-7-1", true, "@1")];
    assert_eq!(attached_counts(&panes, &control).get("@1"), Some(&1));
    assert!(is_own_session("adjterm-7-1") && is_own_session("adjboard-7-1"));
    assert!(!is_own_session("main"));
}

#[test]
fn the_sweep_only_takes_old_unattached_board_sessions_that_share_their_windows() {
    let script = board_sweep_script(Some("/tmp/t/sock"));
    assert!(
        script.contains("tmux -S /tmp/t/sock list-sessions"),
        "{script}"
    );
    assert!(script.contains("adjboard-*|adjterm-*"), "{script}");
    assert!(script.contains("\"$attached\" = 0"), "{script}");
    assert!(script.contains("\"$size\" -gt 1"), "{script}");
    assert!(script.contains("-gt 30"), "{script}");
    assert!(
        script.contains("tmux -S /tmp/t/sock kill-session -t \"=$name\""),
        "{script}"
    );
    assert!(board_sweep_script(Some("adj-test")).contains("tmux -L adj-test kill-session"));
}

#[test]
fn the_release_script_keeps_the_last_holder_of_the_windows() {
    let script = board_release_script(Some("adj-test"), "adjboard-1-2");
    assert!(
        script.contains(
            "tmux -L adj-test display-message -p -t '=adjboard-1-2:' '#{session_group_size}'"
        ),
        "{script}"
    );
    assert!(script.contains("-gt 1"), "{script}");
    assert!(
        script.contains("tmux -L adj-test kill-session -t '=adjboard-1-2'"),
        "{script}"
    );
    assert!(board_release_script(Some("/a/b"), "x").contains("tmux -S /a/b kill-session"));
}

// ── reading a pane before waking it ──────────────────────────────

/// A capture as `tmux_capture_script` prints it, from a real one kept in `src/fixtures`.
fn fixture(name: &str) -> &'static str {
    match name {
        "claude-idle" => include_str!("../fixtures/panes/claude-idle.txt"),
        "claude-idle-after-turn" => include_str!("../fixtures/panes/claude-idle-after-turn.txt"),
        "claude-typing" => include_str!("../fixtures/panes/claude-typing.txt"),
        "claude-working" => include_str!("../fixtures/panes/claude-working.txt"),
        "claude-working-tool" => include_str!("../fixtures/panes/claude-working-tool.txt"),
        "claude-working-typed" => include_str!("../fixtures/panes/claude-working-typed.txt"),
        "claude-question" => include_str!("../fixtures/panes/claude-question.txt"),
        "claude-permission" => include_str!("../fixtures/panes/claude-permission.txt"),
        "claude-menu" => include_str!("../fixtures/panes/claude-menu.txt"),
        "agy-idle" => include_str!("../fixtures/panes/agy-idle.txt"),
        "agy-idle-after-turn" => include_str!("../fixtures/panes/agy-idle-after-turn.txt"),
        "agy-typing" => include_str!("../fixtures/panes/agy-typing.txt"),
        "agy-working" => include_str!("../fixtures/panes/agy-working.txt"),
        "agy-working-typed" => include_str!("../fixtures/panes/agy-working-typed.txt"),
        "agy-question" => include_str!("../fixtures/panes/agy-question.txt"),
        "agy-permission" => include_str!("../fixtures/panes/agy-permission.txt"),
        "agy-menu" => include_str!("../fixtures/panes/agy-menu.txt"),
        other => panic!("no fixture called {other}"),
    }
}

const FIXTURES: &[(Agent, &str, PaneState)] = &[
    (Agent::Claude, "claude-idle", PaneState::Idle),
    (Agent::Claude, "claude-idle-after-turn", PaneState::Idle),
    (Agent::Claude, "claude-typing", PaneState::Typing),
    (Agent::Claude, "claude-working", PaneState::Working),
    (Agent::Claude, "claude-working-tool", PaneState::Working),
    (Agent::Claude, "claude-working-typed", PaneState::Typing),
    (Agent::Claude, "claude-question", PaneState::Asking),
    (Agent::Claude, "claude-permission", PaneState::Asking),
    (Agent::Claude, "claude-menu", PaneState::Asking),
    (Agent::Agy, "agy-idle", PaneState::Idle),
    (Agent::Agy, "agy-idle-after-turn", PaneState::Idle),
    (Agent::Agy, "agy-typing", PaneState::Typing),
    (Agent::Agy, "agy-working", PaneState::Working),
    (Agent::Agy, "agy-working-typed", PaneState::Typing),
    (Agent::Agy, "agy-question", PaneState::Asking),
    (Agent::Agy, "agy-permission", PaneState::Asking),
    (Agent::Agy, "agy-menu", PaneState::Asking),
];

#[test]
fn every_captured_screen_reads_as_what_it_was() {
    for (agent, name, expected) in FIXTURES {
        let screen = parse_pane_screen(fixture(name));
        assert_eq!(pane_state(*agent, &screen), *expected, "{name}");
    }
}

#[test]
fn a_capture_carries_escapes_on_every_line_and_reads_the_same() {
    // The fixtures keep escapes only where the placeholder needs them; a real capture
    // has them everywhere.
    for (agent, name, expected) in FIXTURES {
        let mut screen = parse_pane_screen(fixture(name));
        screen.text = screen
            .text
            .lines()
            .map(|l| format!("\x1b[38;5;244m{l}\x1b[0m"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(pane_state(*agent, &screen), *expected, "{name}");
    }
}

#[test]
fn a_suggestion_in_an_empty_box_is_not_typed_text() {
    // Claude fills an empty box with a suggestion, drawn faint. Read as text it would
    // make every fresh session look like somebody was typing.
    let screen = parse_pane_screen(fixture("claude-idle"));
    assert!(screen.text.contains("Try \""), "{}", screen.text);
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Idle);
    // The same words, not faint, are typed text.
    let mut typed = screen.clone();
    typed.text = typed.text.replace("\x1b[2m", "");
    assert_eq!(pane_state(Agent::Claude, &typed), PaneState::Typing);
}

// ── the last line of output ──────────────────────────────────────

#[test]
fn the_last_line_is_the_one_above_the_input_box() {
    let last = |agent, name| last_output_line(agent, &parse_pane_screen(fixture(name)));
    // Not the box, its border, the prompt or the footer under it; and not the effort level
    // the agent draws at the right of the row above the box.
    assert_eq!(
        last(Agent::Claude, "claude-idle-after-turn").as_deref(),
        Some("✻ Brewed for 3s · done 2:20")
    );
    // Nor the tip it shows under the spinner, or the hint about the terminal it is in.
    assert_eq!(
        last(Agent::Claude, "claude-working").as_deref(),
        Some("✻ Pondering… (3s · thinking)")
    );
    assert_eq!(
        last(Agent::Claude, "claude-working-tool").as_deref(),
        Some("✻ Pondering… (5s · ↓ 264 tokens · thought for 2s)")
    );
    assert_eq!(
        last(Agent::Agy, "agy-working").as_deref(),
        Some("⡿  Generating...")
    );
    assert_eq!(
        last(Agent::Agy, "agy-idle-after-turn").as_deref(),
        Some("rustic appeal, and universally restorative, comforting charm.")
    );
}

#[test]
fn a_screen_without_an_input_box_gives_its_last_line() {
    // A question replaces the box, so there is nothing to look above; its key hints are not
    // the question.
    let question = parse_pane_screen(fixture("claude-question"));
    assert_eq!(
        last_output_line(Agent::Claude, &question).as_deref(),
        Some("4. Chat about this")
    );
    // An agent whose screen is not known has no box to find, whatever it draws.
    let generic = parse_pane_screen("$ make\ncc -o a a.c\n\n  built a\n\n");
    assert_eq!(
        last_output_line(Agent::Generic, &generic).as_deref(),
        Some("built a")
    );
}

#[test]
fn a_line_that_only_mentions_a_key_is_not_a_hint() {
    let screen = parse_pane_screen("building\nPress Esc to cancel the build\n");
    assert_eq!(
        last_output_line(Agent::Generic, &screen).as_deref(),
        Some("Press Esc to cancel the build")
    );
}

#[test]
fn the_last_line_has_no_escapes_and_is_cut_short() {
    let coloured = parse_pane_screen("\x1b[31m\x1b[1merror\x1b[0m: no such file\n");
    assert_eq!(
        last_output_line(Agent::Generic, &coloured).as_deref(),
        Some("error: no such file")
    );
    let long = parse_pane_screen(&format!("{}\n", "あ".repeat(250)));
    let line = last_output_line(Agent::Generic, &long).unwrap();
    assert_eq!(line.chars().count(), LAST_LINE_CHARS + 1);
    assert!(line.ends_with('…'), "{line}");
    // A line of exactly the limit is whole.
    let exact = parse_pane_screen(&format!("{}\n", "a".repeat(LAST_LINE_CHARS)));
    assert_eq!(
        last_output_line(Agent::Generic, &exact).map(|l| l.len()),
        Some(LAST_LINE_CHARS)
    );
}

#[test]
fn a_blank_screen_has_no_last_line() {
    for agent in [Agent::Claude, Agent::Agy, Agent::Generic] {
        let blank = parse_pane_screen("\n  \n\n");
        assert_eq!(last_output_line(agent, &blank), None);
    }
    // Only a box and its footer: nothing was written above it.
    let empty_box = parse_pane_screen(&format!(
        "\n\n{rule}\n❯\n{rule}\n  ? for shortcuts\n",
        rule = "─".repeat(40)
    ));
    assert_eq!(last_output_line(Agent::Claude, &empty_box), None);
}

#[test]
fn a_turn_past_a_minute_is_still_a_turn_in_progress() {
    // The timer runs `3s`, then `1m 5s`, then `1h 2m`.
    let working = fixture("claude-working");
    assert!(working.contains("(3s ·"), "{working}");
    for timer in ["(59s ·", "(1m 5s ·", "(12m ·", "(1h 2m ·"] {
        let mut screen = parse_pane_screen(working);
        screen.text = screen.text.replace("(3s ·", timer);
        assert_eq!(
            pane_state(Agent::Claude, &screen),
            PaneState::Working,
            "{timer}"
        );
    }
    // The line a finished turn leaves has no brackets to read.
    let mut done = parse_pane_screen(working);
    done.text = done
        .text
        .replace("Pondering… (3s · thinking)", "Baked for 1m 5s · done");
    assert_eq!(pane_state(Agent::Claude, &done), PaneState::Idle);
}

#[test]
fn a_spinner_line_before_its_first_tick_is_a_turn_too() {
    let working = fixture("claude-working");
    let mut screen = parse_pane_screen(working);
    screen.text = screen
        .text
        .replace("Pondering… (3s · thinking)", "Pondering…");
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Working);
    // The dot also leads ordinary lines; one ending in an ellipsis is not a spinner.
    let mut listed = parse_pane_screen(working);
    listed.text = listed
        .text
        .replace("✻ Pondering… (3s · thinking)", "· and so on…");
    assert_eq!(pane_state(Agent::Claude, &listed), PaneState::Idle);
}

#[test]
fn the_ascii_spinner_of_other_platforms_is_a_turn_and_a_bullet_is_not() {
    let working = fixture("claude-working");
    let mut star = parse_pane_screen(working);
    star.text = star.text.replace('✻', "*");
    assert_eq!(pane_state(Agent::Claude, &star), PaneState::Working);
    let mut bullet = parse_pane_screen(working);
    bullet.text = bullet
        .text
        .replace("✻ Pondering… (3s · thinking)", "* item in a list");
    assert_eq!(pane_state(Agent::Claude, &bullet), PaneState::Idle);
}

#[test]
fn an_echo_of_an_old_numbered_message_in_the_transcript_is_not_a_question() {
    for (agent, name, echo) in [
        (Agent::Claude, "claude-idle-after-turn", "❯ 1. do this"),
        (Agent::Agy, "agy-idle-after-turn", "> 1. do this"),
    ] {
        let mut screen = parse_pane_screen(fixture(name));
        screen.text = format!("{echo}\n{}", screen.text);
        assert_eq!(pane_state(agent, &screen), PaneState::Idle, "{name}");
    }
    // Typed in the box, it is still somebody typing.
    let mut typing = parse_pane_screen(fixture("claude-typing"));
    typing.text = typing.text.replace("hello typed", "1. foo");
    assert_eq!(pane_state(Agent::Claude, &typing), PaneState::Typing);
    let mut typing = parse_pane_screen(fixture("agy-typing"));
    typing.text = typing.text.replace("hello typed", "1. foo");
    assert_eq!(pane_state(Agent::Agy, &typing), PaneState::Typing);
}

#[test]
fn a_pane_in_copy_mode_is_not_typed_into() {
    for (agent, name, _) in FIXTURES {
        let mut screen = parse_pane_screen(fixture(name));
        screen.in_mode = true;
        assert_eq!(pane_state(*agent, &screen), PaneState::CopyMode, "{name}");
    }
}

#[test]
fn an_agent_of_unknown_kind_is_never_looked_at() {
    for (_, name, _) in FIXTURES {
        let screen = parse_pane_screen(fixture(name));
        assert_eq!(
            pane_state(Agent::Generic, &screen),
            PaneState::Idle,
            "{name}"
        );
    }
}

#[test]
fn a_screen_that_is_not_the_agents_is_unknown() {
    for text in [
        "",
        "$ ",
        "user@host project % ls\nCargo.toml  src\nuser@host project %",
        "❯ not in a box\n",
    ] {
        let screen = parse_pane_screen(&format!("{text}\n{PANE_META_SEPARATOR}\n0\t0\t0\n"));
        for agent in [Agent::Claude, Agent::Agy] {
            assert_eq!(pane_state(agent, &screen), PaneState::Unknown, "{text:?}");
        }
    }
}

#[test]
fn a_prompt_with_a_lot_under_it_is_not_taken_for_the_input_box() {
    let rule = "─".repeat(40);
    let mut text = format!("{rule}\n❯\n{rule}\n");
    for n in 0..=MAX_LINES_BELOW_INPUT {
        text.push_str(&format!("something drawn over it {n}\n"));
    }
    let screen = parse_pane_screen(&format!("{text}{PANE_META_SEPARATOR}\n0\t0\t0\n"));
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Unknown);
}

#[test]
fn a_confirmation_is_a_question_whatever_the_box_says() {
    let rule = "─".repeat(40);
    let text = format!("Overwrite the file? (y/n)\n{rule}\n❯\n{rule}\n");
    let screen = parse_pane_screen(&format!("{text}{PANE_META_SEPARATOR}\n0\t0\t0\n"));
    assert_eq!(pane_state(Agent::Claude, &screen), PaneState::Asking);
}

#[test]
fn the_capture_reads_the_pane_in_utf8_and_keeps_its_attributes() {
    let script = tmux_capture_script(Some("adj-test"), "%3");
    assert!(
        script.starts_with("tmux -u -L adj-test capture-pane -p -e -J -t %3"),
        "{script}"
    );
    assert!(script.contains("display-message -p -t %3"), "{script}");
    assert!(script.contains("#{pane_in_mode}"), "{script}");
    assert!(script.contains(PANE_META_SEPARATOR), "{script}");
    let by_path = tmux_capture_script(Some("/tmp/t.sock"), "%3");
    assert!(
        by_path.starts_with("tmux -u -S /tmp/t.sock capture-pane"),
        "{by_path}"
    );
    assert!(tmux_capture_script(None, "%3").starts_with("tmux -u capture-pane"));
}

#[test]
fn a_capture_is_split_from_what_tmux_says_about_the_pane() {
    let screen = parse_pane_screen(&format!("one\ntwo\n{PANE_META_SEPARATOR}\n1\t12\t3"));
    assert_eq!(screen.text, "one\ntwo");
    assert!(screen.in_mode);
    assert_eq!((screen.cursor_x, screen.cursor_y), (12, 3));
    let bare = parse_pane_screen("one");
    assert!(!bare.in_mode);
    assert_eq!(bare.text, "one");
}

/// A pane the wake looks at, played back: each capture returns the next screen (the last
/// one for as long as it is asked), and everything run is written down.
struct FakePane {
    screens: std::cell::RefCell<std::collections::VecDeque<String>>,
    log: std::cell::RefCell<Vec<String>>,
    waits: std::cell::RefCell<Vec<Duration>>,
}

impl FakePane {
    fn new(screens: &[String]) -> Self {
        FakePane {
            screens: std::cell::RefCell::new(screens.iter().cloned().collect()),
            log: Default::default(),
            waits: Default::default(),
        }
    }

    fn run(&self, cmd: &str) -> Result<String, String> {
        self.log.borrow_mut().push(cmd.to_string());
        if cmd.contains("list-panes") {
            Ok("%1\t12345\t/dev/ttys005\t@1\tadjutant\t1\tworker\n".to_string())
        } else if cmd.contains("capture-pane") {
            let mut screens = self.screens.borrow_mut();
            let next = if screens.len() > 1 {
                screens.pop_front()
            } else {
                screens.front().cloned()
            };
            next.ok_or_else(|| "no screen".to_string())
        } else if cmd.contains("send-keys") {
            Ok(format!("{WOKE_MARKER}\n"))
        } else {
            Err(format!("unexpected command: {cmd}"))
        }
    }

    fn count(&self, needle: &str) -> usize {
        self.log
            .borrow()
            .iter()
            .filter(|c| c.contains(needle))
            .count()
    }

    fn wake(&self, agent: Agent, line: &str) -> Performed {
        let term = TerminalSettings {
            preset: Some("tmux".to_string()),
            ..Default::default()
        };
        let locks = tempfile::tempdir().unwrap();
        wake_with_clock(
            |cmd| self.run(cmd),
            |d| self.waits.borrow_mut().push(d),
            locks.path(),
            Some("ttys005".to_string()),
            &term,
            &Wake::default(),
            &WakeRequest {
                pid: 12345,
                subject: "s",
                line,
                agent,
                dry_run: false,
            },
        )
        .unwrap()
    }
}

/// `screen` with `line` typed at its prompt, as the agent would show it after the keys.
fn with_typed(screen: &str, empty: &str, line: &str) -> String {
    screen.replacen(empty, line, 1)
}

const WAKE: &str =
    "Something arrived in the inbox. Check it with adjutant_pending and deal with it.";

fn claude_idle_with_line_typed() -> String {
    with_typed(fixture("claude-typing"), "hello typed", WAKE)
}

#[test]
fn a_question_on_screen_is_not_answered_by_the_wake() {
    let pane = FakePane::new(&[fixture("claude-question").to_string()]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("question or a menu"),
        "{}",
        done.description
    );
    assert!(
        done.description.contains("not typed"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0, "{:?}", pane.log.borrow());
    // It is waited for, in case the person answers, and then given up on.
    let looks = 1 + (WAKE_READY_BUDGET.as_millis() / WAKE_READY_POLL.as_millis()) as usize;
    assert_eq!(pane.count("capture-pane"), looks);
    assert_eq!(pane.waits.borrow().len(), looks - 1);
}

#[test]
fn a_busy_agent_is_typed_into_once_it_is_back_at_its_prompt() {
    let pane = FakePane::new(&[
        fixture("claude-working").to_string(),
        fixture("claude-idle").to_string(),
        claude_idle_with_line_typed(),
    ]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(done.ran, "{done:?}");
    assert!(
        done.description.contains("woke the session"),
        "{}",
        done.description
    );
    assert_eq!(*pane.waits.borrow(), vec![WAKE_READY_POLL]);
    let log = pane.log.borrow();
    let typed = log.iter().position(|c| c.contains("send-keys -l")).unwrap();
    let enter = log
        .iter()
        .position(|c| c.contains("send-keys -t %1 Enter"))
        .unwrap();
    assert!(typed < enter, "{log:?}");
    assert!(log[typed].contains(WAKE), "{log:?}");
}

#[test]
fn an_agent_busy_for_good_is_looked_at_for_the_whole_budget_and_then_left() {
    let pane = FakePane::new(&[fixture("agy-working").to_string()]);
    let done = pane.wake(Agent::Agy, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("middle of a turn"),
        "{}",
        done.description
    );
    let looks = 1 + (WAKE_READY_BUDGET.as_millis() / WAKE_READY_POLL.as_millis()) as usize;
    assert_eq!(pane.count("capture-pane"), looks);
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn a_person_partway_through_a_message_is_left_alone() {
    let pane = FakePane::new(&[fixture("agy-typing").to_string()]);
    let done = pane.wake(Agent::Agy, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("text typed"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn a_screen_that_is_not_recognised_is_not_typed_into() {
    let pane = FakePane::new(&["$ \n@@adjutant:pane@@\n0\t2\t0\n".to_string()]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("not recognised"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn a_pane_that_cannot_be_read_is_not_typed_into() {
    let pane = FakePane::new(&[]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("could not be read"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
}

#[test]
fn enter_is_not_pressed_when_the_line_did_not_reach_the_prompt() {
    // The screen changed between looking and typing, or the keys went somewhere else:
    // Enter would answer whatever is there.
    let pane = FakePane::new(&[fixture("claude-idle").to_string()]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(
        done.description.contains("Enter was not pressed"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys -l"), 1);
    assert_eq!(pane.count("Enter"), 0);
    assert_eq!(pane.count("capture-pane"), 1 + WAKE_ECHO_LOOKS as usize);
}

#[test]
fn enter_is_not_pressed_when_the_person_started_typing_before_the_line_went_in() {
    // Their text and the line are in the box together; Enter would send both.
    let both = with_typed(
        fixture("claude-typing"),
        "hello typed",
        &format!("hello typed {WAKE}"),
    );
    let pane = FakePane::new(&[fixture("claude-idle").to_string(), both]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(!done.ran, "{done:?}");
    assert!(done.screen, "{done:?}");
    assert!(
        done.description.contains("left at the prompt"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("Enter"), 0);
}

/// One pane shared by wakes running at the same moment: what is typed in its box, and
/// what has been sent from it.
struct SharedPane {
    state: std::sync::Mutex<(String, Vec<String>)>,
}

impl SharedPane {
    fn run(&self, cmd: &str) -> Result<String, String> {
        if cmd.contains("list-panes") {
            return Ok("%1\t12345\t/dev/ttys005\t@1\tadjutant\t1\tworker\n".to_string());
        }
        if cmd.contains("capture-pane") {
            let typed = self.state.lock().unwrap().0.clone();
            // Slow enough for another wake to look in between.
            std::thread::sleep(Duration::from_millis(30));
            return Ok(if typed.is_empty() {
                fixture("claude-idle").to_string()
            } else {
                with_typed(fixture("claude-typing"), "hello typed", &typed)
            });
        }
        let mut state = self.state.lock().unwrap();
        if cmd.contains("send-keys -l") {
            let line = cmd.split('\'').nth(1).unwrap().to_string();
            if !state.0.is_empty() {
                state.0.push(' ');
            }
            state.0.push_str(&line);
        } else if cmd.contains("Enter") {
            let sent = std::mem::take(&mut state.0);
            state.1.push(sent);
            return Ok(format!("{WOKE_MARKER}\n"));
        }
        Ok(String::new())
    }
}

#[test]
fn wakes_at_the_same_moment_take_turns_at_the_pane() {
    let pane = std::sync::Arc::new(SharedPane {
        state: Default::default(),
    });
    let locks = tempfile::tempdir().unwrap();
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    let lines = ["first line from one worker", "second line from another"];
    let results: Vec<Performed> = std::thread::scope(|scope| {
        let handles: Vec<_> = lines
            .iter()
            .map(|line| {
                let (pane, term, locks) = (&pane, &term, locks.path());
                scope.spawn(move || {
                    wake_with_clock(
                        |cmd| pane.run(cmd),
                        std::thread::sleep,
                        locks,
                        Some("ttys005".to_string()),
                        term,
                        &Wake::default(),
                        &WakeRequest {
                            pid: 12345,
                            subject: "s",
                            line,
                            agent: Agent::Claude,
                            dry_run: false,
                        },
                    )
                    .unwrap()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(results.iter().all(|r| r.ran), "{results:?}");
    let mut sent = pane.state.lock().unwrap().1.clone();
    sent.sort();
    assert_eq!(sent, vec![lines[0].to_string(), lines[1].to_string()]);
}

#[test]
fn a_wake_that_cannot_get_the_pane_within_the_budget_gives_up() {
    let pane = FakePane::new(&[fixture("claude-idle").to_string()]);
    let locks = tempfile::tempdir().unwrap();
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    // Someone else holds the pane, as `lock_pane` takes it.
    let mut waited = Duration::ZERO;
    let held = lock_pane(locks.path(), term.tmux_socket(), "%1", &|_| {}, &mut waited)
        .unwrap()
        .unwrap();
    let done = wake_with_clock(
        |cmd| pane.run(cmd),
        |d| pane.waits.borrow_mut().push(d),
        locks.path(),
        Some("ttys005".to_string()),
        &term,
        &Wake::default(),
        &WakeRequest {
            pid: 12345,
            subject: "s",
            line: WAKE,
            agent: Agent::Claude,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(!done.ran && done.screen, "{done:?}");
    assert!(
        done.description.contains("another wake"),
        "{}",
        done.description
    );
    assert_eq!(pane.count("send-keys"), 0);
    assert_eq!(pane.count("capture-pane"), 0);
    let waits = (WAKE_READY_BUDGET.as_millis() / WAKE_READY_POLL.as_millis()) as usize;
    assert_eq!(pane.waits.borrow().len(), waits);
    drop(held);
}

#[test]
fn a_line_wrapped_in_the_box_still_counts_as_shown() {
    let (first, second) = WAKE.split_at(60);
    let rule = "─".repeat(40);
    let wrapped = format!(
        "{rule}\n❯ {first}\n  {second}\n{rule}\n  footer\n{PANE_META_SEPARATOR}\n0\t2\t0\n"
    );
    let pane = FakePane::new(&[fixture("claude-idle").to_string(), wrapped]);
    let done = pane.wake(Agent::Claude, WAKE);
    assert!(done.ran, "{done:?}");
}

#[test]
fn agy_is_typed_into_at_its_prompt_too() {
    let pane = FakePane::new(&[
        fixture("agy-idle").to_string(),
        with_typed(fixture("agy-typing"), "hello typed", WAKE),
    ]);
    let done = pane.wake(Agent::Agy, WAKE);
    assert!(done.ran, "{done:?}");
    assert!(pane.waits.borrow().is_empty());
}

#[test]
fn where_the_screen_is_not_read_nothing_is_captured() {
    // Generic: today's single script, typed without looking.
    let pane = FakePane::new(&[]);
    let done = pane.wake(Agent::Generic, WAKE);
    assert!(done.ran, "{done:?}");
    assert_eq!(pane.count("capture-pane"), 0);
    assert!(done.script.contains("sleep"), "{}", done.script);

    // A dry run only prints.
    let pane = FakePane::new(&[]);
    let term = TerminalSettings {
        preset: Some("tmux".to_string()),
        ..Default::default()
    };
    let locks = tempfile::tempdir().unwrap();
    let done = wake_with_clock(
        |cmd| pane.run(cmd),
        |d| pane.waits.borrow_mut().push(d),
        locks.path(),
        None,
        &term,
        &Wake::default(),
        &WakeRequest {
            pid: 12345,
            subject: "s",
            line: WAKE,
            agent: Agent::Claude,
            dry_run: true,
        },
    );
    // No tty, so the pane is found by pid.
    let done = done.unwrap();
    assert_eq!(pane.count("capture-pane"), 0);
    assert!(done.script.contains("send-keys -l"), "{}", done.script);
    assert!(!done.ran);

    // A template is someone else's command; the built-in for another terminal is too.
    for (wake, terminal) in [
        (
            Wake {
                hook: Hook::Command("tmux send-keys -t {tty} {line} Enter".into()),
                line: None,
            },
            term.clone(),
        ),
        (Wake::default(), TerminalSettings::default()),
    ] {
        let pane = FakePane::new(&[]);
        let locks = tempfile::tempdir().unwrap();
        let done = wake_with_clock(
            |cmd| {
                pane.log.borrow_mut().push(cmd.to_string());
                Ok(WOKE_MARKER.to_string())
            },
            |d| pane.waits.borrow_mut().push(d),
            locks.path(),
            Some("ttys005".to_string()),
            &terminal,
            &wake,
            &WakeRequest {
                pid: 12345,
                subject: "s",
                line: WAKE,
                agent: Agent::Claude,
                dry_run: false,
            },
        )
        .unwrap();
        assert!(done.ran, "{done:?}");
        assert_eq!(pane.count("capture-pane"), 0, "{:?}", pane.log.borrow());
        assert!(pane.waits.borrow().is_empty());
    }
}
