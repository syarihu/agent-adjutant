//! The subcommands. This is the only layer allowed to know about more than one of the
//! others: everything below is a leaf that answers one question, and joining them up is
//! what a command is.

use serde_json::{Value, json};
use std::io::Read;

use crate::infra::ide;
use crate::infra::notify;
use crate::infra::terminal::{self, SpawnRequest};
use crate::kernel::config::{self, Settings};
use crate::kernel::identity;
use crate::mail::{self, Message, NotWoken, Reached, deliver_to_hub_with_wake, deliver_to_worker};
use crate::registry::{self, Context, context, context_as, context_without_hub, resolve};

mod board_actions;
mod board_terminal;
mod close;
mod context;
mod delivery;
mod gate;
mod gh;
mod hub_ops;
mod hub_title;
mod ide_title_notify;
mod jules;
mod pr_poll;
mod review_engine;
mod serve;
mod session;
mod skill_outbox;
mod stdin;
mod task;
mod tell;
pub mod tmux;
mod worker_ops;

pub use close::close;
use context::settings_for;
pub use context::{hub_name, show_config};
pub(crate) use delivery::wake_note_sentence;
pub use delivery::{PendingArgs, SendArgs, pending, send};
pub use gate::{
    AnswerArgs, CloseArgs, answer_cmd as gate_answer, close_cmd as gate_close, list as gate_list,
    open_cmd as gate_open, open_json as gate_open_json, show as gate_show,
};
use hub_ops::{ago, print_performed};
pub use hub_ops::{hub, hub_close, hub_stop};
pub use hub_title::HubTitles;
pub use ide_title_notify::{WorktreeArgs, notify_user, open_ide, set_title, worktree_path};
pub use jules::{
    ShowArgs as JulesShowArgs, StartArgs as JulesStartArgs, Watch as JulesWatch,
    findings_cmd as jules_findings_cmd, relay_cmd as jules_relay_cmd, show as jules_show,
    start as jules_start,
};
pub use pr_poll::PrPoll;
pub use review_engine::run as review_engine;
pub use serve::{
    DEFAULT_PORT, HubBoard, board_json, serve, serve_for_hub, server_restart, server_start,
    server_status, server_stop,
};
#[cfg(test)]
pub(crate) use skill_outbox::skill_text;
pub use skill_outbox::{outbox, skill};
use stdin::{dash_is_stdin, read_body};
pub use task::{
    AddArgs, BriefArgs, UpdateArgs, add as task_add, brief as task_brief,
    fetch_issue_cmd as task_fetch_issue_cmd, list as task_list, next_cmd as task_next,
    refresh_cmd as task_refresh_cmd, refresh_json as task_refresh_json, show as task_show,
    update_cmd as task_update,
};
pub use tell::{TellArgs, tell};
pub use worker_ops::{WorkArgs, WorkerArgs, focus, focus_worker_cmd, phase, spawn, work, worker};

#[cfg(test)]
mod tests;
