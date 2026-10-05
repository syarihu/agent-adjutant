//! The command line: read the arguments clap parsed, call an operation, print what came back.
//! Each file is one command or a family of them; `run` takes the parsed arguments apart and
//! calls them.

use serde_json::{Value, json};
use std::io::Read;

use crate::infra::ide;
use crate::infra::notify;
use crate::infra::terminal::{self, SpawnRequest};
use crate::kernel::config::Settings;
use crate::kernel::identity;
use crate::mail::{self, Message, NotWoken, Reached, deliver_to_hub_with_wake, deliver_to_worker};
use crate::registry::{self, Context, context, context_as, context_without_hub, resolve};
use crate::transport::wording::wake_note_sentence;

mod args;
mod close;
mod config;
mod delivery;
mod gate;
mod hub;
mod ide_title_notify;
mod jules;
mod review_engine;
mod run;
mod serve;
mod server;
mod skill_outbox;
mod stdin;
mod task;
mod tell;
pub mod tmux;
mod worker;

pub use close::close;
use config::settings_for;
pub use config::{hub_name, show_config};
pub use delivery::{pending, send};
pub use gate::{
    answer_cmd as gate_answer, close_cmd as gate_close, list as gate_list, open_cmd as gate_open,
    show as gate_show,
};
use hub::{ago, print_performed};
pub use hub::{hub, hub_close, hub_stop};
pub use ide_title_notify::{notify_user, open_ide, set_title, worktree_path};
pub use jules::{
    findings_cmd as jules_findings_cmd, relay_cmd as jules_relay_cmd, show as jules_show,
    start as jules_start,
};
pub use review_engine::run as review_engine;
pub use run::run;
pub use serve::serve;
pub use server::{server_restart, server_start, server_status, server_stop};
pub use skill_outbox::{outbox, skill};
use stdin::{dash_is_stdin, read_body};
pub use task::{
    add as task_add, brief as task_brief, fetch_issue_cmd as task_fetch_issue_cmd,
    list as task_list, next_cmd as task_next, refresh_cmd as task_refresh_cmd, show as task_show,
    update_cmd as task_update,
};
pub use tell::tell;
pub use worker::{WorkArgs, WorkerArgs, focus, focus_worker_cmd, phase, spawn, work, worker};
