//! Issues a bearer token for one source (K6/AR17). Prints the
//! plaintext token once — to hand to that source — and the SHA-256
//! hash to paste into its profile's `token_hash`. Almanac itself never
//! stores the plaintext, so a leaked profile file grants nothing and
//! this output is the only chance to copy the token.
//!
//! Deliberately a separate tool rather than a server endpoint: minting
//! credentials is a setup action, not something a running service
//! should expose.
//!
//! Usage:
//!   cargo run --example issue_token -- home-assistant

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(source_id) = std::env::args().nth(1) else {
        eprintln!("usage: issue_token <source_id>");
        eprintln!("       names the profile this token is for, e.g. home-assistant");
        return ExitCode::FAILURE;
    };

    // 32 bytes of OS randomness, hex-encoded: long enough that
    // guessing is hopeless, and safe to paste into a shell or a YAML
    // file without quoting surprises.
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS refused to provide randomness");
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hash = almanac::core::token::hash_token(&token);

    println!("Token for source \"{source_id}\" — shown once, copy it now:");
    println!();
    println!("  {token}");
    println!();
    println!("Put this in the source's request:");
    println!("  Authorization: Bearer {token}");
    println!();
    println!("Put this in profiles/{source_id}.toml (safe to commit):");
    println!("  token_hash = \"{hash}\"");

    ExitCode::SUCCESS
}
