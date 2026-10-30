//! The older environment names Almanac still honours or refuses, each
//! with the warning or remedy that says so (standing rule 12: no silent
//! substitution).

use std::collections::BTreeMap;

use almanac::shell::admin::{BOOTSTRAP_TOKEN_ENV, CAPTURE_TOKEN_ENV};

/// The 2.x names, mapped onto the kit's knobs on the environment snapshot
/// (never `set_var`, which is unsound once threads exist). Returns what to
/// say about it once logging is up — standing rule 12: no silent
/// substitution.
pub fn compat(env: &mut BTreeMap<String, String>) -> Vec<String> {
    let mut warnings = Vec::new();
    if !env.contains_key("ALMANAC_LISTEN")
        && let Some(bind) = env.get("ALMANAC_BIND").cloned()
    {
        env.insert("ALMANAC_LISTEN".to_string(), bind);
        warnings.push(
            "ALMANAC_BIND is the 2.x name of ALMANAC_LISTEN; it still works, rename it in the \
             environment file — the alias goes away in 4.0"
                .to_string(),
        );
    }
    if !env.contains_key("ALMANAC_UPDATE_MODE")
        && let Some(raw) = env.get("ALMANAC_SELF_UPDATE").cloned()
    {
        let mode = match raw.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "1" | "yes" => "autonomous",
            _ => "off",
        };
        env.insert("ALMANAC_UPDATE_MODE".to_string(), mode.to_string());
        warnings.push(format!(
            "ALMANAC_SELF_UPDATE={raw} is the 2.x knob; it now means ALMANAC_UPDATE_MODE={mode} \
             (off | supervised | autonomous) — set that instead, the alias goes away in 4.0"
        ));
    }
    if let Some(url) = env.get("ALMANAC_UPDATE_URL").cloned() {
        let trimmed = url.trim_end_matches('/');
        if trimmed.ends_with("/releases") {
            let fixed = format!("{trimmed}/latest/download");
            env.insert("ALMANAC_UPDATE_URL".to_string(), fixed.clone());
            warnings.push(format!(
                "ALMANAC_UPDATE_URL={url} has the 2.x shape; the kit wants the directory holding \
                 VERSION and SHA256SUMS, so {fixed} is used — set that, or unset it to derive it \
                 from the repository"
            ));
        }
    }
    if !env.contains_key("ALMANAC_LOG")
        && let Some(filter) = env.get("RUST_LOG").cloned()
    {
        // A convention rather than an Almanac knob until 3.0.0; honoured quietly.
        env.insert("ALMANAC_LOG".to_string(), filter);
    }
    warnings
}

/// 4.0.0: the login token is the kit's ALMANAC_TOKEN. A 3.x environment
/// file still naming the old variable is refused with the rename spelled
/// out; the capture token is simply no longer read, which `warnings`
/// gains a line about.
pub fn check_retired_tokens(
    env: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let set = |key: &str| env.get(key).is_some_and(|v| !v.trim().is_empty());
    if set(BOOTSTRAP_TOKEN_ENV) {
        return Err(format!(
            "{BOOTSTRAP_TOKEN_ENV} is the 3.x name; 4.0.0 reads ALMANAC_TOKEN (the kit's login token)\n  remedy: rename the variable in the environment file (latch.env on CT 112) and restart"
        ));
    }
    if set(CAPTURE_TOKEN_ENV) {
        warnings.push(format!(
            "{CAPTURE_TOKEN_ENV} is no longer read since 4.0.0: a system that posts captures gets a client token from the Sources page instead; remove the variable"
        ));
    }
    Ok(())
}
