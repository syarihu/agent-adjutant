use crate::board::Server;
use crate::mail::{self, RepoHub};

/// The hub `id` names. `id` is a `hubs[].id` the page was given, so the page can only name a
/// hub this repository was found to have.
pub fn find(server: &Server, id: &str) -> Result<RepoHub, String> {
    mail::all_repo_hubs(&server.ctx.state, &server.ctx.repo)
        .into_iter()
        .find(|h| h.id == id)
        .ok_or_else(|| format!("no such hub: {id}"))
}
