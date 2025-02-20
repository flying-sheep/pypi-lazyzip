use std::collections::HashMap;

use color_eyre::eyre::Result;
use either::Either;
use reqwest::Url;
use serde::{Deserialize, Deserializer};
use serde_with::{DisplayFromStr, serde_as};

/// A project on the simple API.
/// See [spec](https://packaging.python.org/en/latest/specifications/simple-repository-api/#project-detail).
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Project {
    pub meta: Meta,
    pub name: String,
    pub files: Vec<File>,
}

/// Project metadata on the simple API.
#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Meta {
    pub api_version: String,
}

/// A file on the simple API.
#[allow(dead_code)]
#[serde_as]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct File {
    pub filename: String,
    #[serde_as(as = "DisplayFromStr")]
    pub url: Url,
    pub hashes: HashMap<String, String>,
    pub requires_python: Option<String>,
    #[serde(default)]
    pub core_metadata: CoreMetadata,
    #[serde(default)]
    pub gpg_sig: bool,
    #[serde(default)]
    pub yanked: Yanking,
}

/// Indicator if the (wheel) file has core metadata.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum CoreMetadata {
    /// The file has core metadata.
    /// It might have associated hashes
    /// See [available hashes](https://docs.python.org/3/library/hashlib.html)
    Present(Box<Hashes>),
    /// The file does not have core metadata.
    #[default]
    Absent,
}

/// Available hashes, all might be None.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct Hashes {
    sha1: Option<String>,
    sha224: Option<String>,
    sha256: Option<String>,
    sha384: Option<String>,
    sha512: Option<String>,
    sha3_224: Option<String>,
    sha3_256: Option<String>,
    sha3_384: Option<String>,
    sha3_512: Option<String>,
    shake_128: Option<String>,
    shake_256: Option<String>,
    blake2b: Option<String>,
    blake2s: Option<String>,
    md5: Option<String>,
}

impl<'de> Deserialize<'de> for CoreMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let reason: Option<Either<Hashes, bool>> =
            either::serde_untagged_optional::deserialize(deserializer)?;
        Ok(match reason {
            Some(Either::Left(hashes)) => CoreMetadata::Present(Box::new(hashes)),
            Some(Either::Right(true)) => CoreMetadata::Present(Box::default()),
            Some(Either::Right(false)) | None => CoreMetadata::Absent,
        })
    }
}

/// Is a package yanked or not?
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Yanking {
    /// Yanked, optionally with a reason
    Yanked(Option<String>),
    /// Not yanked
    #[default]
    NotYanked,
}

impl std::ops::Not for &Yanking {
    type Output = bool;

    fn not(self) -> Self::Output {
        matches!(self, &Yanking::NotYanked)
    }
}

impl<'de> Deserialize<'de> for Yanking {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let reason: Option<Either<String, bool>> =
            either::serde_untagged_optional::deserialize(deserializer)?;
        Ok(match reason {
            Some(Either::Left(reason)) => Yanking::Yanked(Some(reason)),
            Some(Either::Right(true)) => Yanking::Yanked(None),
            Some(Either::Right(false)) | None => Yanking::NotYanked,
        })
    }
}
