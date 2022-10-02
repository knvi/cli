use anyhow::Result;
use clap::Parser;

use crate::{
    commands::channels::tokens::utils::parse_expiration, state::State, util::validate_json,
};

use super::utils::create_token;

#[derive(Debug, Parser, Default, PartialEq, Eq)]
#[clap(about = "Create a new Leap Token")]
pub struct Options {
    #[clap(
        short = 'e',
        long = "expiration",
        help = "Expiration date of the token, can be a date (DD/MM/YYYY, DD-MM-YYYY) or a duration (60s, 1d, 30d, 1y)",
        validator = parse_expiration
    )]
    pub expires_at: Option<String>,

    #[clap(
        short = 's',
        long = "state",
        help = "Initial state of the token, can be any JSON value",
        validator = validate_json
    )]
    pub state: Option<String>,
}

pub async fn handle(options: Options, state: State) -> Result<()> {
    let project_id = state.ctx.current_project_error().id;

    let (token_state, expires_at) = if options != Options::default() {
        (
            options.state,
            options.expires_at.map(|ex| parse_expiration(&ex).unwrap()),
        )
    } else {
        let token_state = dialoguer::Input::<String>::new()
            .with_prompt("State")
            .default("null".to_string())
            .validate_with(|s: &String| validate_json(s))
            .interact_text()?;

        let expires_at = dialoguer::Input::<String>::new()
            .with_prompt("Expiration date")
            .default("0".to_string())
            .validate_with(|s: &String| parse_expiration(s).map(|_| ()))
            .interact_text()?;

        (
            if token_state.to_lowercase() == "null" {
                None
            } else {
                Some(token_state)
            },
            if expires_at.to_lowercase() == "0" {
                None
            } else {
                Some(parse_expiration(&expires_at).unwrap())
            },
        )
    };

    let token = create_token(
        &state.http,
        &project_id,
        expires_at.as_deref(),
        token_state.map(|s| s.parse().unwrap()),
    )
    .await?;

    log::info!("Created Token: `{}`", token.id);

    Ok(())
}
