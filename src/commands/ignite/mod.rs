pub mod create;
mod delete;
mod list;
pub mod rollout;
mod scale;
pub mod types;
mod update;
pub mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};

use self::create::{handle as handle_create, Options as CreateOptions};
use self::delete::{handle as handle_delete, Options as DeleteOptions};
use self::list::{handle as handle_list, Options as ListOptions};
use self::rollout::{handle as handle_rollout, Options as RolloutOptions};
use self::scale::{handle as handle_scale, Options as ScaleOptions};
use self::update::{handle as handle_update, Options as UpdateOptions};
use crate::state::State;

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[clap(name = "new", alias = "create")]
    Create(CreateOptions),
    #[clap(name = "ls", alias = "list")]
    List(ListOptions),
    #[clap(name = "rm", alias = "delete")]
    Delete(DeleteOptions),
    #[clap(name = "rollout", alias = "rollouts")]
    Rollout(RolloutOptions),
    #[clap(name = "update")]
    Update(UpdateOptions),
    #[clap(name = "scale")]
    Scale(ScaleOptions),
}

#[derive(Debug, Parser)]
#[clap(name = "ignite", about = "Interact with Ignite containers")]
pub struct Options {
    #[clap(subcommand)]
    pub commands: Commands,
}

pub async fn handle(options: Options, state: State) -> Result<()> {
    match options.commands {
        Commands::List(options) => handle_list(options, state).await,
        Commands::Create(options) => handle_create(options, state).await,
        Commands::Delete(options) => handle_delete(options, state).await,
        Commands::Update(options) => handle_update(options, state).await,
        Commands::Rollout(options) => handle_rollout(options, state).await,
        Commands::Scale(options) => handle_scale(options, state).await,
    }
}
