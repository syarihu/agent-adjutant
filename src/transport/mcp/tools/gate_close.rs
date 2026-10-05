use crate::gate::close as gate_close_payload;
use serde_json::{Value, json};

use super::resolve_repo;

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let id = args["id"].as_str().ok_or("a gate needs an id")?;
    let comment = args.get("comment").and_then(Value::as_str);
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
    let terminal = args
        .get("terminal")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let gate = gate_close_payload(&ctx, id, comment, terminal)?;
    let on_board = gate.answered_on_board();
    Ok(json!({ "gate": gate, "closed": !on_board, "alreadyAnswered": on_board }))
}
