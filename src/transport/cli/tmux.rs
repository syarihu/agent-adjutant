//! Tmux backend CLI subcommands: inspecting panes, waking, focusing, closing, and spawning.

use serde_json::json;

use super::args::{TmuxCloseArgs, TmuxFocusArgs, TmuxPaneArgs, TmuxSpawnArgs, TmuxWakeArgs};
use super::settings_for;
use crate::infra::terminal;

pub fn pane(args: &TmuxPaneArgs) -> Result<(), String> {
    let (pid, tty) = (args.pid, args.tty.as_deref());
    let as_json = args.json;
    let settings = settings_for(args.repo.as_deref());
    let socket = args
        .socket
        .as_deref()
        .or_else(|| settings.terminal.tmux_socket());
    if pid.is_some() || tty.is_some() {
        let pane = terminal::find_tmux_pane(socket, pid, tty)?;
        match pane {
            Some(pane) => {
                if as_json {
                    println!("{}", json!(pane));
                } else {
                    println!(
                        "{} (pid {}, {}, window {}, session {}, {})",
                        pane.pane_id,
                        pane.pane_pid,
                        pane.pane_tty,
                        pane.window_id,
                        pane.session_name,
                        pane.window_name
                    );
                }
            }
            None => {
                if as_json {
                    println!("null");
                } else {
                    let desc = match (pid, tty) {
                        (Some(p), Some(t)) => format!("pid {p} / tty {t}"),
                        (Some(p), None) => format!("pid {p}"),
                        (None, Some(t)) => format!("tty {t}"),
                        (None, None) => unreachable!(),
                    };
                    return Err(format!("no tmux pane found for {desc}"));
                }
            }
        }
    } else {
        let panes = terminal::list_tmux_panes(socket)?;
        if as_json {
            println!("{}", json!(panes));
        } else {
            for p in &panes {
                println!(
                    "{} (pid {}, {}, window {}, session {}, {})",
                    p.pane_id, p.pane_pid, p.pane_tty, p.window_id, p.session_name, p.window_name
                );
            }
        }
    }
    Ok(())
}

pub fn wake(args: &TmuxWakeArgs) -> Result<(), String> {
    let dry_run = args.dry_run;
    let settings = settings_for(args.repo.as_deref());
    let socket = args
        .socket
        .as_deref()
        .or_else(|| settings.terminal.tmux_socket());
    // Generic unless told: a pane whose screen is not known is typed into without looking,
    // as it always was, and looking is what the caller asks for by naming the agent.
    let agent = args
        .agent
        .as_deref()
        .and_then(crate::infra::agent::Agent::parse)
        .unwrap_or(crate::infra::agent::Agent::Generic);
    let performed = terminal::tmux_wake(
        socket,
        args.pid,
        args.line.as_deref(),
        crate::mail::look_before_typing(agent),
        dry_run,
    )?;
    if dry_run {
        println!("{}", performed.script);
        return Ok(());
    }
    if !performed.ran {
        return Err(performed.description);
    }
    println!("{}", performed.description);
    Ok(())
}

pub fn focus(args: &TmuxFocusArgs) -> Result<(), String> {
    let dry_run = args.dry_run;
    let settings = settings_for(args.repo.as_deref());
    let socket = args
        .socket
        .as_deref()
        .or_else(|| settings.terminal.tmux_socket());
    let performed = terminal::tmux_focus(socket, args.pid, dry_run)?;
    if dry_run {
        println!("{}", performed.script);
        return Ok(());
    }
    if !performed.ran {
        return Err(performed.description);
    }
    println!("{}", performed.description);
    Ok(())
}

pub fn close(args: &TmuxCloseArgs) -> Result<(), String> {
    let dry_run = args.dry_run;
    let settings = settings_for(args.repo.as_deref());
    let socket = args
        .socket
        .as_deref()
        .or_else(|| settings.terminal.tmux_socket());
    let performed = terminal::tmux_close(socket, args.pid, dry_run)?;
    if dry_run {
        println!("{}", performed.script);
        return Ok(());
    }
    if !performed.ran {
        return Err(performed.description);
    }
    println!("{}", performed.description);
    Ok(())
}

pub fn spawn(args: &TmuxSpawnArgs) -> Result<(), String> {
    let command = args.command();
    let dry_run = args.dry_run;
    if command.is_empty() {
        return Err("pass the command to run after --".to_string());
    }
    let settings = settings_for(args.repo.as_deref());
    let socket = args
        .socket
        .as_deref()
        .or_else(|| settings.terminal.tmux_socket());
    let session = args
        .session
        .as_deref()
        .or_else(|| Some(settings.terminal.tmux_session()));
    let expanded;
    let cwd_str = match args.cwd.as_deref() {
        Some(c) => {
            expanded = crate::infra::paths::expand_home(c);
            expanded.to_string_lossy()
        }
        None => std::borrow::Cow::Borrowed("."),
    };
    let performed =
        terminal::tmux_spawn(socket, session, &cwd_str, &args.title, &command, dry_run)?;
    if dry_run {
        println!("{}", performed.script);
        return Ok(());
    }
    println!("{}", performed.description);
    Ok(())
}
