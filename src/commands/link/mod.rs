use std::env::current_dir;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::commands::ignite::util::{format_deployments, get_all_deployments};
use crate::state::State;
use crate::store::hopfile::HopFile;

#[derive(Debug, Parser)]
#[structopt(about = "Link an existing deployment to a hopfile")]
pub struct Options {
    #[structopt(
        name = "dir",
        help = "Directory to link, defaults to current directory"
    )]
    path: Option<PathBuf>,
    #[structopt(short = 'n', long = "name", help = "Name of the deployment")]
    name: Option<String>,
}

pub async fn handle(options: Options, state: State) -> Result<()> {
    let mut dir = current_dir().expect("Could not get current directory");

    if let Some(path) = options.path {
        dir = dir
            .join(path)
            .canonicalize()
            .expect("Could not get canonical path");
    }

    assert!(dir.is_dir(), "{} is not a directory", dir.display());

    if HopFile::find(dir.clone()).await.is_some() {
        log::warn!("A hopfile was found in {}", dir.display());
    }

    let project = state.ctx.current_project_error();

    log::info!(
        "Project: {} /{} ({})",
        project.name,
        project.namespace,
        project.id
    );

    let deployments = get_all_deployments(&state.http, &project.id).await?;

    assert!(
        !deployments.is_empty(),
        "No deployments found in this project"
    );

    let deployment = match options.name {
        Some(name_or_id) => {
            let deployment = deployments
                .iter()
                .find(|d| d.id == name_or_id || d.name == name_or_id)
                .expect("Deployment not found");
            log::info!("Deployment: {} ({})", deployment.name, deployment.id);

            deployment
        }
        None => {
            let deployments_fmt = format_deployments(&deployments, false);

            let idx = dialoguer::Select::new()
                .with_prompt("Select a deployment to link")
                .items(&deployments_fmt)
                .default(0)
                .interact_opt()
                .expect("Failed to select deployment")
                .expect("No deployment selected");

            &deployments[idx]
        }
    };

    HopFile::new(dir.join("hop.yml"), project.id, deployment.id.clone())
        .save()
        .await?;

    log::info!(
        "Deployment `{}` ({}) linked",
        deployment.name,
        deployment.id
    );

    Ok(())
}
