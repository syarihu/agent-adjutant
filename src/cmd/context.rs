use super::*;

// ── hub-name ─────────────────────────────────────────────────────────

pub fn hub_name(
    repo_arg: Option<&str>,
    hub_arg: Option<&str>,
    as_json: bool,
) -> Result<(), String> {
    let info = resolve(repo_arg, hub_arg)?;
    if as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "main": info.main,
                "nwo": info.nwo,
                "repo": info.repo,
                // Which hub of the repository this address belongs to, as it was resolved
                // — `null` for the repository's own. Printed because it is the only way to
                // see, from outside, which of the three answers won.
                "hub": info.hub,
                "slug": info.slug,
                "hubName": info.hub_name,
                "nwoSource": info.nwo_source,
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    if info.nwo_source == "dirname" {
        eprintln!(
            "adjutant: origin gave no repository name, using the directory name {}",
            info.nwo
        );
    }
    println!("{}", info.hub_name);
    Ok(())
}

// ── config ───────────────────────────────────────────────────────────

pub fn show_config(repo_arg: Option<&str>, hub_arg: Option<&str>) -> Result<(), String> {
    let ctx = context(repo_arg, hub_arg)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "repo": ctx.repo.nwo,
            "main": ctx.repo.main,
            "hub": ctx.repo.hub,
            "hubName": ctx.repo.hub_name,
            "board": board_json(&ctx.state, &ctx.repo),
            "registered": ctx.resolved.registered,
            "configPath": ctx.resolved.config_path,
            "warnings": ctx.resolved.warnings,
            "settings": ctx.settings,
            "config": ctx.resolved.config,
        }))
        .unwrap_or_default()
    );
    Ok(())
}

/// This binary, for commands that have to name themselves in a command line handed to a
/// terminal. The absolute path rather than `adjutant`, so a new tab whose PATH is not yet
/// loaded still finds it.
pub(super) fn exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "adjutant".to_string())
}

/// Settings without insisting on a resolvable repository.
///
/// `spawn` and `notify` are useful from anywhere, including outside a checkout, and failing
/// them because `git` had nothing to say would be answering a question nobody asked.
pub(super) fn settings_for(repo_arg: Option<&str>) -> Settings {
    let nwo = match repo_arg {
        Some(arg) => arg.to_string(),
        None => identity::resolve(None, None)
            .map(|i| i.nwo)
            .unwrap_or_default(),
    };
    config::resolve_config(&nwo)
        .map(|r| r.settings)
        .unwrap_or_default()
}
