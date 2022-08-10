use anyhow::{bail, ensure, Result};
use clap::Parser;

use crate::commands::containers::utils::create_containers;
use crate::commands::ignite::util::{format_deployments, get_all_deployments};
use crate::state::State;

#[derive(Debug, Parser)]
#[clap(about = "Create containers for a deployment")]
pub struct Options {
    #[clap(
        short = 'd',
        long = "deployment",
        help = "NAME or ID of the deployment"
    )]
    pub deployment: Option<String>,

    #[clap(name = "count", help = "Number of containers to create")]
    pub count: Option<u64>,
}

pub async fn handle(options: Options, state: State) -> Result<()> {
    let project_id = state.ctx.current_project_error().id;

    let deployments = get_all_deployments(&state.http, &project_id).await?;

    ensure!(!deployments.is_empty(), "No deployments found");

    let deployment = match options.deployment {
        Some(name) => {
            let deployment = deployments
                .iter()
                .find(|p| p.name == name || p.id == name)
                .expect("Deployment not found");

            deployment.clone()
        }
        None => {
            let deployments_fmt = format_deployments(&deployments, false);

            let idx = dialoguer::Select::new()
                .with_prompt("Select a deployment to delete")
                .items(&deployments_fmt)
                .default(0)
                .interact_opt()
                .expect("Failed to select deployment")
                .expect("No deployment selected");

            deployments[idx].clone()
        }
    };

    let count = match options.count {
        Some(count) => count,
        None => dialoguer::Input::<u64>::new()
            .with_prompt("Number of containers to create")
            .interact()
            .expect("Failed to select deployment"),
    };

    if count < 1 {
        bail!("Count must be greater than 0");
    }

    create_containers(&state.http, &deployment.id, count).await?;

    log::info!("Created {} containers", count);

    Ok(())
}
