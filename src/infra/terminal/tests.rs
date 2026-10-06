use super::*;
use crate::infra::terminal::Hook;

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
        None,
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
        None,
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
        None,
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
        None,
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
            look: None,
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
            look: None,
            dry_run: false,
        },
    )
    .unwrap();
    assert!(woken.ran, "{woken:?}");

    // A configured template is answered for by its exit status: it is someone else's
    // command and only it knows what success means there.
    let template = Wake {
        hook: crate::infra::terminal::Hook::Command("true {pid}".to_string()),
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
            look: None,
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
            look: None,
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
fn the_prepare_script_makes_a_grouped_session_and_removes_it_if_it_missed_the_window() {
    let script = board_attach_prepare_script(Some("/tmp/t/sock"), "work", "adjboard-1-2", "@5");
    assert_eq!(
        script,
        "tmux -S /tmp/t/sock list-sessions >/dev/null && \
         tmux -S /tmp/t/sock new-session -d -s adjboard-1-2 -t work && \
         { tmux -S /tmp/t/sock select-window -t '=adjboard-1-2:@5' || \
         { tmux -S /tmp/t/sock kill-session -t '=adjboard-1-2' 2>/dev/null; false; }; }"
    );
    assert!(!script.contains("select-pane"), "{script}");
    assert!(!script.contains("window-size"), "{script}");
    assert!(!script.contains("ignore-size"), "{script}");
    assert!(!script.contains("destroy-unattached"), "{script}");
    // No hooks: one on a session that is destroyed later crashed the tmux server.
    assert!(!script.contains("set-hook"), "{script}");
    // The group name need not be a session's name: a group outlives its first session.
    assert!(!script.contains("has-session"), "{script}");
    // `=` would make a new group named `=work` rather than join `work`.
    assert!(!script.contains("-t =work"), "{script}");

    let spaced = board_attach_prepare_script(Some("adj-test"), "my session", "adjboard-1-2", "@5");
    assert!(
        spaced.contains(
            "tmux -L adj-test new-session -d -s adjboard-1-2 -t 'my session' && \
             { tmux -L adj-test select-window"
        ),
        "{spaced}"
    );
}

#[test]
fn only_a_window_or_server_that_is_gone_reads_as_gone() {
    assert!(is_gone_error("can't find window: @5"));
    assert!(is_gone_error("no server running on /tmp/tmux-501/x"));
    assert!(is_gone_error(
        "error connecting to /tmp/tmux-501/x (No such file or directory)"
    ));
    assert!(is_gone_error("Can't find session: x"));
    assert!(!is_gone_error("duplicate session: adjboard-1-2"));
    assert!(!is_gone_error(""));
}

/// A tmux server of its own for running the prepare script against, and everything it made
/// gone when this is dropped. `None` where there is no tmux.
struct PrepareTmux {
    socket: String,
    cwd: tempfile::TempDir,
}

impl PrepareTmux {
    fn new(start: bool) -> Option<Self> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let found = std::process::Command::new("tmux").arg("-V").output().ok()?;
        if !found.status.success() {
            return None;
        }
        let this = PrepareTmux {
            // The shape the test sweep of `tests/common` recognises, should a kill be missed.
            socket: format!(
                "adj-test-prepare-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            // Under TMPDIR, so that the leak check sees a server that outlives its test.
            cwd: tempfile::tempdir().unwrap(),
        };
        if start {
            let out = this.tmux(&[
                "-f",
                "/dev/null",
                "start-server",
                ";",
                "set",
                "-s",
                "exit-empty",
                "off",
                ";",
                "set",
                "-g",
                "default-shell",
                "/bin/sh",
                ";",
                "set",
                "-g",
                "default-command",
                "exec cat",
            ]);
            assert!(out.status.success(), "{out:?}");
        }
        Some(this)
    }

    fn tmux(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new("tmux")
            .current_dir(self.cwd.path())
            .args(["-L", &self.socket])
            .args(args)
            .output()
            .unwrap()
    }

    fn out(&self, args: &[&str]) -> String {
        let out = self.tmux(args);
        assert!(
            out.status.success(),
            "tmux {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn sessions(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .out(&["list-sessions", "-F", "#{session_name}"])
            .lines()
            .map(str::to_string)
            .collect();
        names.sort();
        names
    }

    fn prepare(&self, group: &str, name: &str, window: &str) -> Result<String, String> {
        crate::infra::shell::run_shell(&board_attach_prepare_script(
            Some(&self.socket),
            group,
            name,
            window,
        ))
    }
}

impl Drop for PrepareTmux {
    fn drop(&mut self) {
        // Whatever the test did, the server (if any) goes, and its shells with it.
        let _ = self.tmux(&["kill-server"]);
    }
}

#[test]
fn preparing_a_session_in_a_group_that_is_gone_does_not_start_a_server() {
    let Some(tmux) = PrepareTmux::new(false) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    let err = tmux.prepare("work", "adjboard-1-1", "@0").unwrap_err();
    assert!(
        err.contains("no server running") || err.contains("error connecting"),
        "{err}"
    );
    assert!(is_gone_error(&err), "{err}");
    assert!(!tmux.tmux(&["list-sessions"]).status.success());
}

#[test]
fn preparing_a_session_in_a_group_that_is_gone_removes_the_stray_it_made() {
    let Some(tmux) = PrepareTmux::new(true) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    tmux.out(&["new-session", "-d", "-s", "other"]);
    let window = tmux.out(&["display-message", "-p", "-t", "=other:", "#{window_id}"]);
    let err = tmux.prepare("G", "adjboard-1-1", &window).unwrap_err();
    assert!(err.contains("can't find"), "{err}");
    assert!(is_gone_error(&err), "{err}");
    assert_eq!(tmux.sessions(), ["other"]);
}

#[test]
fn a_group_whose_first_session_is_gone_is_still_joined() {
    let Some(tmux) = PrepareTmux::new(true) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    tmux.out(&["new-session", "-d", "-s", "work", "-n", "main"]);
    let window = tmux.out(&[
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{window_id}",
        "-t",
        "=work:",
        "-n",
        "target",
    ]);
    tmux.out(&["new-session", "-d", "-s", "m2", "-t", "work"]);
    tmux.out(&["kill-session", "-t", "=work"]);
    assert!(!tmux.tmux(&["has-session", "-t", "=work"]).status.success());

    tmux.prepare("work", "adjboard-1-1", &window).unwrap();
    assert_eq!(tmux.sessions(), ["adjboard-1-1", "m2"]);
    let made = "=adjboard-1-1:";
    assert_eq!(
        tmux.out(&["display-message", "-p", "-t", made, "#{session_group}"]),
        "work"
    );
    assert_eq!(
        tmux.out(&["display-message", "-p", "-t", made, "#{session_group_size}"]),
        "2"
    );
    assert_eq!(
        tmux.out(&["display-message", "-p", "-t", made, "#{window_id}"]),
        window
    );
}

#[test]
fn a_name_that_is_taken_fails_without_touching_the_session_that_has_it() {
    let Some(tmux) = PrepareTmux::new(true) else {
        eprintln!("tmux not available, skipping test");
        return;
    };
    tmux.out(&["new-session", "-d", "-s", "work"]);
    let window = tmux.out(&["display-message", "-p", "-t", "=work:", "#{window_id}"]);
    tmux.out(&["new-session", "-d", "-s", "adjboard-1-1", "-t", "work"]);
    let err = tmux.prepare("work", "adjboard-1-1", &window).unwrap_err();
    assert!(err.contains("duplicate session"), "{err}");
    assert!(!is_gone_error(&err), "{err}");
    assert_eq!(tmux.sessions(), ["adjboard-1-1", "work"]);
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
