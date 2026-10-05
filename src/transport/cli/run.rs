//! The dispatch: each clap variant taken apart and handed to the command that runs it.

use clap::Parser;

use super::args::{
    Cli, Commands, GateAction, JulesAction, ServerAction, TaskAction, TmuxAction, strip_separator,
};
use super::outbox;
use crate::transport::cli;

/// Parse this process's arguments and run the subcommand. Exits; never returns.
pub fn run() -> ! {
    crate::kernel::config::anchor_config_env();
    let cli = Cli::parse();
    let result: Result<i32, String> = match &cli.command {
        Commands::HubName(args) => super::hub_name(args).map(|_| 0),
        Commands::Config(args) => super::show_config(args).map(|_| 0),
        Commands::Pending(args) => super::pending(args).map(|_| 0),
        Commands::Send(args) => super::send(args).map(|_| 0),
        Commands::Spawn {
            repo,
            cwd,
            title,
            dry_run,
            command,
        } => super::spawn(
            repo.as_deref(),
            cwd,
            title,
            &strip_separator(command),
            *dry_run,
        )
        .map(|_| 0),
        Commands::Work {
            repo,
            hub,
            worktree,
            title,
            task,
            prompt,
            resume,
            dry_run,
        } => super::work(&super::WorkArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            worktree,
            title,
            task: task.as_deref(),
            prompt: prompt.as_deref(),
            resume: *resume,
            dry_run: *dry_run,
        }),
        // Exit 1 when the hub is not running, so a shell can branch on it without parsing
        // anything this prints.
        Commands::Worker {
            repo,
            hub,
            worktree,
            title,
            task,
            prompt,
            resume,
            dry_run,
        } => super::worker(&super::WorkerArgs {
            repo: repo.as_deref(),
            hub: hub.as_deref(),
            worktree: worktree.as_deref(),
            title: title.as_deref(),
            task: task.as_deref(),
            prompt: prompt.as_deref(),
            resume: *resume,
            dry_run: *dry_run,
        })
        .map(|_| 0),
        Commands::Tell(args) => super::tell(args).map(|_| 0),
        Commands::Serve(args) => super::serve(args).map(|_| 0),
        Commands::Server { action } => match action {
            ServerAction::Start(args) => super::server_start(args),
            ServerAction::Stop => super::server_stop(),
            ServerAction::Restart(args) => super::server_restart(args),
            ServerAction::Status(args) => super::server_status(args),
        },
        Commands::Gate { action } => run_gate(action).map(|_| 0),
        Commands::Task { action } => run_task(action).map(|_| 0),
        Commands::Jules { action } => run_jules(action).map(|_| 0),
        Commands::Skill(args) => super::skill(args).map(|_| 0),
        Commands::Outbox(args) => outbox(args).map(|_| 0),
        Commands::Focus {
            repo,
            hub: _,
            worktree: Some(worktree),
            quiet,
            dry_run,
        } => super::focus_worker_cmd(repo.as_deref(), worktree, *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Focus {
            repo,
            hub,
            worktree: None,
            quiet,
            dry_run,
        } => super::focus(repo.as_deref(), hub.as_deref(), *quiet, *dry_run)
            .map(|found| i32::from(!found)),
        Commands::Phase { set, worktree } => {
            super::phase(worktree.as_deref(), set.as_deref()).map(|_| 0)
        }
        Commands::ReviewEngine(args) => super::review_engine(args).map(|_| 0),
        Commands::Close {
            repo,
            worktree,
            quiet,
            dry_run,
        } => {
            cli::close(repo.as_deref(), worktree, *quiet, *dry_run).map(|closed| i32::from(!closed))
        }
        Commands::Hub {
            repo,
            hub,
            dry_run,
            tab,
            resume,
            new,
            no_dashboard,
            dashboard,
            extra,
        } => super::hub(
            repo.as_deref(),
            hub.as_deref(),
            &strip_separator(extra),
            *tab,
            match (*resume, *new) {
                (true, _) => crate::lifecycle::hub::HubStart::Resume,
                (_, true) => crate::lifecycle::hub::HubStart::New,
                _ => crate::lifecycle::hub::HubStart::Auto,
            },
            // Two flags, three answers. `None` is "nobody said", and it has to stay distinct
            // from both: it is what leaves the configured value standing, and what keeps a
            // plain `adj hub` printing the command line it has always printed.
            match (*no_dashboard, *dashboard) {
                (true, _) => Some(false),
                (_, true) => Some(true),
                _ => None,
            },
            *dry_run,
        )
        .map(|_| 0),
        Commands::HubStop { repo, hub } => {
            super::hub_stop(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::HubClose { repo, hub } => {
            super::hub_close(repo.as_deref(), hub.as_deref()).map(|_| 0)
        }
        Commands::Ide(args) => super::open_ide(args).map(|_| 0),
        Commands::Title(args) => super::set_title(args).map(|_| 0),
        Commands::Notify(args) => super::notify_user(args).map(|_| 0),
        Commands::WorktreePath(args) => super::worktree_path(args).map(|_| 0),
        Commands::Mcp => crate::transport::mcp::run_server()
            .map(|_| 0)
            .map_err(|e| e.to_string()),
        Commands::InstallMcp(args) => crate::transport::mcp::install(&args.target).map(|_| 0),
        Commands::UninstallMcp(args) => crate::transport::mcp::uninstall(&args.target).map(|_| 0),
        Commands::Tmux { action } => run_tmux(action).map(|_| 0),
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("adjutant: {message}");
            std::process::exit(1);
        }
    }
}

/// The `adj task` verbs. Split out of `run` because the verbs are a subcommand of a
/// subcommand, and inlining that match would bury the rest of the dispatch.
fn run_task(action: &TaskAction) -> Result<(), String> {
    match action {
        TaskAction::Add(args) => super::task_add(args),
        TaskAction::List(args) => super::task_list(args),
        TaskAction::Show(args) => super::task_show(args),
        TaskAction::Next(args) => super::task_next(args),
        TaskAction::Refresh(args) => super::task_refresh_cmd(args),
        TaskAction::FetchIssue(args) => super::task_fetch_issue_cmd(args),
        TaskAction::Update(args) => super::task_update(args),
        TaskAction::Brief(args) => super::task_brief(args),
    }
}

/// The `adj jules` verbs, split out for the same reason `run_task` is.
fn run_jules(action: &JulesAction) -> Result<(), String> {
    match action {
        JulesAction::Start(args) => super::jules_start(args),
        JulesAction::Show(args) => super::jules_show(args),
        JulesAction::Findings(args) => super::jules_findings_cmd(args),
        JulesAction::Relay(args) => super::jules_relay_cmd(args),
    }
}

/// The `adj gate` verbs, split out for the same reason `run_task` is.
fn run_gate(action: &GateAction) -> Result<(), String> {
    match action {
        GateAction::Open(args) => super::gate_open(args),
        GateAction::List(args) => super::gate_list(args),
        GateAction::Show(args) => super::gate_show(args),
        GateAction::Answer(args) => super::gate_answer(args),
        GateAction::Close(args) => super::gate_close(args),
    }
}

/// The `adj tmux` verbs.
fn run_tmux(action: &TmuxAction) -> Result<(), String> {
    match action {
        TmuxAction::Pane(args) => super::tmux::pane(args),
        TmuxAction::Wake(args) => super::tmux::wake(args),
        TmuxAction::Focus(args) => super::tmux::focus(args),
        TmuxAction::Close(args) => super::tmux::close(args),
        TmuxAction::Spawn(args) => super::tmux::spawn(args),
    }
}
