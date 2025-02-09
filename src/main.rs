use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::LazyLock};

use async_http_range_reader::{AsyncHttpRangeReader, CheckSupportMethod};
use async_zip::base::read::seek::ZipFileReader;
use clap::Parser;
use color_eyre::eyre::{bail, Context, ContextCompat, Error, OptionExt, Result};
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
    filename: String,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct PackageName(String);

/// Regex matching strings that start with valid Python package identifiers
static ID_START_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)^[A-Z0-9][A-Z0-9._-]*[A-Z0-9]|[A-Z0-9]").unwrap());

impl FromStr for PackageName {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // If the match is not as long as the whole thing, there is more after
        if !ID_START_RE.find(s).is_some_and(|m| m.len() == s.len()) {
            bail!("invalid identifier");
        }
        Ok(PackageName(caseless::default_case_fold_str(s)))
    }
}

impl Into<String> for PackageName {
    fn into(self) -> String {
        self.0
    }
}

impl ToString for PackageName {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WheelFilename {
    pub name: PackageName,
    pub version: pep440_rs::Version,
    tags: String, // e.g. "py3-none-any"
}

impl FromStr for WheelFilename {
    type Err = Error;

    fn from_str(filename: &str) -> Result<Self, Self::Err> {
        let stem = filename
            .strip_suffix(".whl")
            .ok_or_eyre("not a .whl file")?;
        let &[name, version, tags] = stem.splitn(3, '-').collect::<Vec<_>>().as_slice() else {
            bail!("invalid wheel filename: {stem}");
        };
        Ok(WheelFilename {
            name: PackageName::from_str(name)?,
            version: pep440_rs::Version::from_str(version)?,
            tags: tags.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
enum PkgLoc {
    Name(PackageName, Option<pep440_rs::VersionSpecifier>),
    Path(PathBuf),
}

impl PkgLoc {
    fn from_name(s: &str) -> Result<Self> {
        let Some(name) = ID_START_RE.find(s) else {
            bail!("invalid identifier");
        };
        let rest = &s[name.len()..];
        let version_spec = (!rest.is_empty())
            .then(|| pep440_rs::VersionSpecifier::from_str(&rest))
            .transpose()
            .with_context(|| format!("could not parse version from {rest}"))?;
        Ok(PkgLoc::Name(
            PackageName::from_str(name.as_str())?,
            version_spec,
        ))
    }
}

impl FromStr for PkgLoc {
    type Err = Error;

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
        PkgLoc::Name(name, version_spec) => {
            let client = reqwest::Client::new(); //.builder().http2_prior_knowledge().build()?
            let pkg: Package = client
                .get(format!("https://pypi.org/simple/{}/", name.to_string()))
                .header("Accept", "application/vnd.pypi.simple.v1+json")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            let mut whls: Vec<_> = pkg
                .files
                .into_iter()
                .filter_map(|p| {
                    let n = WheelFilename::from_str(&p.filename).ok()?;
                    if version_spec
                        .as_ref()
                        .is_none_or(|version_spec| version_spec.contains(&n.version))
                    {
                        Some((n, p))
                    } else {
                        None
                    }
                })
                .collect();
            whls.sort_by(|(name_l, _), (name_r, _)| name_l.version.cmp(&name_r.version));
            let (_, whl) = whls.drain(..).rev().next().context("No wheel found")?;
            dbg!(&whl.filename);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str() {
        assert!(matches!(PkgLoc::from_str("foo"), Ok(PkgLoc::Name(_, None))));
        assert!(matches!(
            PkgLoc::from_str("foo==1.0"),
            Ok(PkgLoc::Name(_, _))
        ));
        assert!(matches!(
            PkgLoc::from_str("foo ==1.0.1"),
            Ok(PkgLoc::Name(_, _))
        ));
        // all else are path
    }
}
