use color_eyre::eyre::{Context as _, Error};

use crate::pkg_name::PackageName;

mod spec;

pub use spec::*;

pub async fn fetch_project(client: &reqwest::Client, name: &PackageName) -> Result<Project, Error> {
    client
        .get(format!("https://pypi.org/simple/{name}/"))
        .header("Accept", "application/vnd.pypi.simple.v1+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("Failed to parse JSON")
}
