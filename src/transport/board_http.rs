//! The board's HTTP and WebSocket side: parse the request, call a board, task, gate or Jules
//! operation, and word the answer. The board's servers are handed `handle` (a hub's board or
//! a dedicated `adj serve`) and `handle_resident` (the resident's) as their connection handler.

mod assets;
mod auth;
mod handlers;
mod resident;
mod routes;
mod terminal;

pub use resident::handle_resident;
pub use routes::handle;

#[cfg(test)]
mod tests;
