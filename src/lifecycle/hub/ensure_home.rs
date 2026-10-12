use crate::registry::Context;

/// Make the directory the hub starts in, when it is one adj owns: the code-review hub's home,
/// `<state>/review`, which belongs to no checkout and so is not there until something makes it.
/// A repository's hub starts in its main checkout, which is never made here.
pub fn ensure_home(ctx: &Context) -> Result<(), String> {
    if !ctx.repo.is_review() {
        return Ok(());
    }
    std::fs::create_dir_all(&ctx.repo.main)
        .map_err(|e| format!("cannot create {}: {e}", ctx.repo.main))
}
