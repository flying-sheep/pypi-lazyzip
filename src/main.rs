use std::{collections::HashMap, ffi::OsStr, path::PathBuf, str::FromStr};

use async_zip::base::read::seek::ZipFileReader;
use color_eyre::eyre::{ContextCompat, Result};
use reqwest::Url;
use serde::{de::Error as _, Deserialize, Deserializer};
use tokio::io::{AsyncBufRead, AsyncRead, AsyncSeek};

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

struct RangeReader {
    client: reqwest::Client,
    url: Url,
    content_length: Option<u64>,
    offset: u64,
}
impl RangeReader {
    async fn new(client: reqwest::Client, url: Url) -> Result<Self> {
        let head = client.head(url.clone()).send().await?.error_for_status()?;

        Ok(Self {
            client,
            url,
            content_length: head
                .headers()
                .get("content-length")
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.parse().ok()),
            offset: 0,
        })
    }
}

impl AsyncRead for RangeReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        dbg!(self.offset, self.content_length);
        todo!()
    }
}

impl AsyncSeek for RangeReader {
    fn start_seek(
        self: std::pin::Pin<&mut Self>,
        position: std::io::SeekFrom,
    ) -> std::io::Result<()> {
        self.get_mut().offset = match position {
            std::io::SeekFrom::Start(offset) => offset,
            std::io::SeekFrom::Current(offset) => self.offset.saturating_add_signed(offset),
            std::io::SeekFrom::End(offset) => self
                .content_length
                .ok_or_else(|| std::io::ErrorKind::InvalidInput)?
                .saturating_add_signed(offset),
        };
        Ok(())
    }

    fn poll_complete(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<u64>> {
        std::task::Poll::Ready(Ok(self.offset))
    }
}

impl AsyncBufRead for RangeReader {
    fn poll_fill_buf(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<&[u8]>> {
        todo!()
    }

    fn consume(self: std::pin::Pin<&mut Self>, amt: usize) {
        todo!()
    }
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

    let reader = RangeReader::new(client, whl.url).await?;
    let mut r = ZipFileReader::with_tokio(reader).await?;
    let idx_entry = r
        .file()
        .entries()
        .into_iter()
        .enumerate()
        .find(|(_, e)| e.filename().as_str().ok() == Some("top_level.txt"))
        .map(|(i, _)| i)
        .context("No top_level.txt file found")?;

    let mut buf = String::new();
    r.reader_with_entry(idx_entry)
        .await?
        .read_to_string_checked(&mut buf)
        .await?;
    Ok(())
}
