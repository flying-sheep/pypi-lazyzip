use std::{collections::HashMap, ffi::OsStr, path::PathBuf, str::FromStr, sync::LazyLock};

use async_http_range_reader::{AsyncHttpRangeReader, CheckSupportMethod};
use async_zip::base::read::seek::ZipFileReader;
use clap::Parser;
use color_eyre::eyre::{bail, Context, ContextCompat, Result};
use reqwest::{header::HeaderMap, Url};
use serde::{de::Error as _, Deserialize, Deserializer};
use tokio::io::{AsyncRead, AsyncSeek};

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct Package {
    meta: Meta,
    name: String,
    files: Vec<File>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct Meta {
    api_version: String,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct File {
    filename: PathBuf,
    #[serde(deserialize_with = "deserialize_url")]
    url: Url,
    hashes: HashMap<String, String>,
    requires_python: Option<String>,
    #[serde(default)]
    dist_info_metadata: bool,
    #[serde(default)]
    gpg_sig: bool,
    /// `Some(maybe_reason)` if yanked, else `None`
    #[serde(default, deserialize_with = "reason")]
    yanked: Option<Option<String>>,
}

fn deserialize_url<'de, D>(deserializer: D) -> Result<Url, D::Error>
where
    D: Deserializer<'de>,
{
    let url: &str = Deserialize::deserialize(deserializer)?;
    url.parse().map_err(D::Error::custom)
}

fn reason<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let reason: Option<either::Either<String, bool>> =
        either::serde_untagged_optional::deserialize(deserializer)?;
    Ok(reason.and_then(|maybe_reason| match maybe_reason {
        either::Either::Left(reason) => Some(Some(reason)),
        either::Either::Right(true) => Some(None),
        either::Either::Right(false) => None,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Identifier(String);

/// Regex matching strings that start with valid Python package identifiers
static ID_START_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)^[A-Z0-9][A-Z0-9._-]*[A-Z0-9]|[A-Z0-9]").unwrap());

impl FromStr for Identifier {
    type Err = color_eyre::eyre::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // If the match is not as long as the whole thing, there is more after
        if !ID_START_RE.find(s).is_some_and(|m| m.len() == s.len()) {
            bail!("invalid identifier");
        }
        Ok(Identifier(caseless::default_case_fold_str(s)))
    }
}

impl Into<String> for Identifier {
    fn into(self) -> String {
        self.0
    }
}

impl ToString for Identifier {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

#[derive(Debug, Clone)]
enum PkgLoc {
    Name(Identifier, Option<pep440_rs::Version>),
    Path(PathBuf),
}

impl PkgLoc {
    fn from_name(s: &str) -> Result<Self> {
        let Some(name) = ID_START_RE.find(s) else {
            bail!("invalid identifier");
        };
        let rest = &s[name.len()..];
        let version = (!rest.is_empty())
            .then(|| pep440_rs::Version::from_str(rest))
            .transpose()
            .with_context(|| format!("could not parse version from {rest}"))?;
        Ok(PkgLoc::Name(Identifier::from_str(name.as_str())?, version))
    }
}

impl FromStr for PkgLoc {
    type Err = color_eyre::eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_name(s).or_else(|_| Ok(PkgLoc::Path(PathBuf::from(s))))
    }
}

#[derive(clap::Parser)]
struct Cli {
    pkg_loc: PkgLoc,
}

trait AsyncRS: AsyncRead + AsyncSeek + Unpin {}

impl<R> AsyncRS for R where R: AsyncRead + AsyncSeek + Unpin {}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Cli::try_parse()?;

    let reader: Box<dyn AsyncRS> = match args.pkg_loc {
        PkgLoc::Name(name, version) => {
            if !version.is_none() {
                todo!();
            }
            let client = reqwest::Client::new(); //.builder().http2_prior_knowledge().build()?
            let pkg: Package = client
                .get(format!("https://pypi.org/simple/{}/", name.to_string()))
                .header("Accept", "application/vnd.pypi.simple.v1+json")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let whl = pkg
                .files
                .into_iter()
                .find(|p| p.filename.extension() == Some(OsStr::new("whl")))
                .context("No .whl file found")?;

            let (reader, _headers) = AsyncHttpRangeReader::new(
                client,
                whl.url,
                CheckSupportMethod::Head,
                HeaderMap::new(),
            )
            .await?;
            Box::new(reader)
        }
        PkgLoc::Path(path) => {
            let reader = tokio::fs::File::open(path).await?;
            Box::new(reader)
        }
    };
    let buf_reader = tokio::io::BufReader::new(reader);
    let mut r = ZipFileReader::with_tokio(buf_reader).await?;
    let idx_entry = r
        .file()
        .entries()
        .into_iter()
        .enumerate()
        .find(|(_, e)| {
            e.filename()
                .as_str()
                .is_ok_and(|n| n.ends_with("/top_level.txt"))
        })
        .map(|(i, _)| i)
        .context("No top_level.txt file found")?;

    let mut buf = String::new();
    r.reader_with_entry(idx_entry)
        .await?
        .read_to_string_checked(&mut buf)
        .await?;
    println!("{buf}");
    Ok(())
}
