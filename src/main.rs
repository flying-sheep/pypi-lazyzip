use std::{collections::HashMap, ffi::OsStr, path::PathBuf};

use async_http_range_reader::{AsyncHttpRangeReader, CheckSupportMethod};
use async_zip::base::read::seek::ZipFileReader;
use color_eyre::eyre::{ContextCompat, Result};
use reqwest::{header::HeaderMap, Url};
use serde::{de::Error as _, Deserialize, Deserializer};

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

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let pkg_name = "torch";

    let client = reqwest::Client::new(); //.builder().http2_prior_knowledge().build()?
    let pkg: Package = client
        .get(format!("https://pypi.org/simple/{pkg_name}/"))
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

    let (reader, _headers) =
        AsyncHttpRangeReader::new(client, whl.url, CheckSupportMethod::Head, HeaderMap::new())
            .await?;
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
