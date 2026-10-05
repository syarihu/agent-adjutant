use crate::task::refresh as task_refresh;
use crate::transport::wording::refresh_json as task_refresh_json;
use serde_json::Value;

use super::resolve_repo;

pub(in crate::transport::mcp) fn call(args: &Value) -> Result<Value, String> {
    let ctx = crate::registry::context_of(resolve_repo(args)?)?;
    let checked = task_refresh(&ctx)?;
    Ok(task_refresh_json(&checked))
}
