//! What the board does by itself, beside answering requests: the poll of GitHub's
//! notifications and of the pull requests the cards hold, the parent-task titles of the hubs,
//! and what Jules says about the sessions the cards follow, and the sweep that closes the gates
//! whose worker has moved on.

mod hub_titles;
mod jules_watch;
mod notifications;
mod pr_poll;
pub mod sweep_gates;

pub use hub_titles::{HubTitles, cached_title};
pub use jules_watch::{JulesSeen, Watch};
pub use pr_poll::{PollHealth, PrPoll};
