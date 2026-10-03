//! Tmux backend CLI subcommands: inspecting panes, waking, focusing, closing, and spawning.

use serde_json::json;

use crate::cmd::settings_for;
use crate::terminal;

pub fn pane(
    repo: Option<&str>,
    socket: Option<&str>,
    pid: Option<u32>,
    tty: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let settings = settings_for(repo);
    let socket = socket.or_else(|| settings.terminal.tmux_socket());
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

pub fn wake(
    repo: Option<&str>,
    socket: Option<&str>,
    pid: u32,
    line: Option<&str>,
    agent: Option<&str>,
    dry_run: bool,
) -> Result<(), String> {
    let settings = settings_for(repo);
    let socket = socket.or_else(|| settings.terminal.tmux_socket());
    // Generic unless told: a pane whose screen is not known is typed into without looking,
    // as it always was, and looking is what the caller asks for by naming the agent.
    let agent = agent
        .and_then(crate::prompts::Agent::parse)
        .unwrap_or(crate::prompts::Agent::Generic);
    let performed = terminal::tmux_wake(
        socket,
        pid,
        line,
        terminal::look_before_typing(agent),
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

pub fn focus(
    repo: Option<&str>,
    socket: Option<&str>,
    pid: u32,
    dry_run: bool,
) -> Result<(), String> {
    let settings = settings_for(repo);
    let socket = socket.or_else(|| settings.terminal.tmux_socket());
    let performed = terminal::tmux_focus(socket, pid, dry_run)?;
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

pub fn close(
    repo: Option<&str>,
    socket: Option<&str>,
    pid: u32,
    dry_run: bool,
) -> Result<(), String> {
    let settings = settings_for(repo);
    let socket = socket.or_else(|| settings.terminal.tmux_socket());
    let performed = terminal::tmux_close(socket, pid, dry_run)?;
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

pub fn spawn(
    repo: Option<&str>,
    socket: Option<&str>,
    session: Option<&str>,
    cwd: Option<&str>,
    title: &str,
    command: &[String],
    dry_run: bool,
) -> Result<(), String> {
    if command.is_empty() {
        return Err("pass the command to run after --".to_string());
    }
    let settings = settings_for(repo);
    let socket = socket.or_else(|| settings.terminal.tmux_socket());
    let session = session.or_else(|| Some(settings.terminal.tmux_session()));
    let expanded;
    let cwd_str = match cwd {
        Some(c) => {
            expanded = crate::config::expand_home(c);
            expanded.to_string_lossy()
        }
        None => std::borrow::Cow::Borrowed("."),
    };
    let performed = terminal::tmux_spawn(socket, session, &cwd_str, title, command, dry_run)?;
    if dry_run {
        println!("{}", performed.script);
        return Ok(());
    }
    println!("{}", performed.description);
    Ok(())
}
