#![deny(clippy::pedantic)]

use std::{path::PathBuf, str::FromStr};

use async_http_range_reader::{AsyncHttpRangeReader, CheckSupportMethod};
use async_zip::base::read::seek::ZipFileReader;
use clap::Parser;
use color_eyre::eyre::{Context as _, ContextCompat, Error, Result};
use reqwest::header::HeaderMap;
use tokio::io::{AsyncRead, AsyncSeek};
use tracing::instrument::Instrument as _;
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;

use crate::pkg_name::{parse_dependency, PackageName, WheelFilename};
use crate::simple_repo_api::fetch_project;

mod pkg_name;
mod simple_repo_api;

#[derive(Debug, Clone)]
enum PkgLoc {
    Name(PackageName, Option<pep440_rs::VersionSpecifier>),
    Path(PathBuf),
}

impl FromStr for PkgLoc {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok((name, version_spec)) = parse_dependency(s) {
            Ok(PkgLoc::Name(name, version_spec))
        } else {
            Ok(PkgLoc::Path(PathBuf::from(s)))
        }
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
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE | FmtSpan::NEW)
        .with_writer(std::io::stderr)
        .init();

    let args = Cli::try_parse()?;

    let reader: Box<dyn AsyncRS> = match args.pkg_loc {
        PkgLoc::Name(name, version_spec) => {
            let client = reqwest::Client::new(); //.builder().http2_prior_knowledge().build()?
            let whl = find_wheel(&client, name, version_spec)
                .instrument(tracing::info_span!("find_wheel"))
                .await?;
            let (reader, _headers) = AsyncHttpRangeReader::new(
                client,
                whl.url,
                CheckSupportMethod::Head,
                HeaderMap::new(),
            )
            .instrument(tracing::info_span!("create_range_reader"))
            .await?;
            Box::new(reader)
        }
        PkgLoc::Path(path) => {
            let reader = tokio::fs::File::open(path).await?;
            Box::new(reader)
        }
    };
    let buf_reader = tokio::io::BufReader::new(reader);
    let mut zip_reader = ZipFileReader::with_tokio(buf_reader)
        .instrument(tracing::info_span!("create_zip_reader"))
        .await?;
    let idx_entry = zip_reader
        .file()
        .entries()
        .iter()
        .enumerate()
        .find(|(_, e)| {
            e.filename()
                .as_str()
                .is_ok_and(|n| n.ends_with("/top_level.txt"))
        })
        .map(|(i, _)| i)
        .context("No top_level.txt file found")?;

    let mut buf = String::new();
    read_entry(&mut zip_reader, idx_entry, &mut buf).await?;
    println!("{buf}");
    Ok(())
}

async fn find_wheel(
    client: &reqwest::Client,
    name: PackageName,
    version_spec: Option<pep440_rs::VersionSpecifier>,
) -> Result<simple_repo_api::File> {
    fetch_project(client, &name)
        .await?
        .files
        .into_iter()
        .filter_map(|p| {
            let n = WheelFilename::from_str(&p.filename).ok()?;
            if !&p.yanked
                && version_spec
                    .as_ref()
                    .is_none_or(|version_spec| version_spec.contains(&n.version))
            {
                Some((n, p))
            } else {
                None
            }
        })
        .max_by(|(name_l, _), (name_r, _)| name_l.version.cmp(&name_r.version))
        .map(|(_, whl)| whl)
        .with_context(|| format!("No wheel found for {name} {version_spec:?}"))
}

#[tracing::instrument(skip(reader, buf))]
async fn read_entry<R>(reader: &mut ZipFileReader<R>, idx: usize, buf: &mut String) -> Result<usize>
where
    R: futures_lite::io::AsyncBufRead + futures_lite::io::AsyncSeek + Unpin,
{
    reader
        .reader_with_entry(idx)
        .instrument(tracing::info_span!("create_entry_reader"))
        .await?
        .read_to_string_checked(buf)
        .instrument(tracing::info_span!("read_to_string"))
        .await
        .context("Failed to read entry")
}
