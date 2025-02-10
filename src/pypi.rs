use std::collections::HashMap;

use color_eyre::eyre::Result;
use reqwest::Url;
use serde::{de::Error as _, Deserialize, Deserializer};

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Package {
    pub meta: Meta,
    pub name: String,
    pub files: Vec<File>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Meta {
    pub api_version: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct File {
    pub filename: String,
    #[serde(deserialize_with = "deserialize_url")]
    pub url: Url,
    pub hashes: HashMap<String, String>,
    pub requires_python: Option<String>,
    #[serde(default)]
    pub dist_info_metadata: bool,
    #[serde(default)]
    pub gpg_sig: bool,
    #[serde(default, deserialize_with = "reason")]
    pub yanked: Yanking,
}

fn deserialize_url<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let url: &str = Deserialize::deserialize(deserializer)?;
    url.parse().map_err(D::Error::custom)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Yanking {
    Yanked(Option<String>),
    #[default]
    NotYanked,
}

impl std::ops::Not for &Yanking {
    type Output = bool;

    fn not(self) -> Self::Output {
        matches!(self, &Yanking::NotYanked)
    }
}

fn reason<'de, D>(deserializer: D) -> Result<Yanking, D::Error>
where
    D: Deserializer<'de>,
{
    let reason: Option<either::Either<String, bool>> =
        either::serde_untagged_optional::deserialize(deserializer)?;
    let Some(maybe_reason) = reason else {
        return Ok(Yanking::NotYanked);
    };
    Ok(match maybe_reason {
        either::Either::Left(reason) => Yanking::Yanked(Some(reason)),
        either::Either::Right(true) => Yanking::Yanked(None),
        either::Either::Right(false) => Yanking::NotYanked,
    })
}
