#![warn(clippy::pedantic)]

use anyhow::Result;
use clap::Parser;
use hop_cli::commands::update::util::version_notice;
use hop_cli::commands::{handle_command, Commands};
use hop_cli::state::{State, StateOptions};
use hop_cli::{utils, CLI};

#[tokio::main]
async fn main() -> Result<()> {
    // setup panic hook
    utils::set_hook();

    // create a new CLI instance
    let cli = CLI::parse();

    utils::logs(cli.verbose);

    let state = State::new(StateOptions {
        override_project_id: cli.project,
        override_token: std::env::var("HOP_TOKEN").ok(),
    })
    .await;

    match cli.commands {
        Commands::Update(_) => None,
        // its okay for the notice to fail
        _ => version_notice(state.ctx.clone()).await.ok(),
    };

    if let Err(error) = handle_command(cli.commands, state).await {
        log::error!("{}", error);
        std::process::exit(1);
    }

    Ok(())
}
