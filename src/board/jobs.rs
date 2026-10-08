//! What the board does by itself, beside answering requests: the poll of GitHub's
//! notifications and of the pull requests the cards hold, the parent-task titles of the hubs,
//! and what Jules says about the sessions the cards follow, the sweep that closes the gates
//! whose worker has moved on, and the watch that announces a session waiting on a person.

mod branch_prs;
mod hub_titles;
mod issue_parents;
mod jules_watch;
mod notifications;
mod pr_poll;
pub mod sweep_gates;
mod wait_watch;

pub use hub_titles::{HubTitles, cached_title};
pub use jules_watch::{JulesSeen, Watch};
pub use pr_poll::{HOST, PollBoards, PollHealth, PrPoll};
pub use wait_watch::{OpenGuard, WaitWatch, target_key};
