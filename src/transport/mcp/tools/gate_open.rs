use crate::gate::open as gate_open_payload;
use crate::kernel::identity;
use crate::transport::wording::open_json as gate_open_json;
use serde_json::{Value, json};

use super::{cwd_param, resolve_repo};

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
    let mut payload = args.clone();
    let fields = payload
        .as_object_mut()
        .ok_or("arguments must be an object")?;
    for key in ["repo", "hub", "cwd"] {
        fields.remove(key);
    }
    // From the caller's `cwd`, for the reason `adjutant_send` gives: the server's own
    // directory is not where the worker is standing, and this is where the answer goes.
    // A client that fills in every advertised property sends `null` or `""` for one it
    // has no value for; either is as good as absent.
    let given = fields
        .get("worktree")
        .and_then(Value::as_str)
        .is_some_and(|w| !w.trim().is_empty());
    if !given {
        let here = identity::current_worktree(cwd_param(args).as_deref())
            .ok_or("not inside a worktree: pass worktree or cwd")?;
        fields.insert("worktree".to_string(), json!(here));
    }
    let (gate, served) = gate_open_payload(&ctx, crate::gate::GateRequest::from_json(&payload)?)?;
    Ok(gate_open_json(&ctx, &gate, served))
}
