use std::{path::PathBuf, str::FromStr};

use async_http_range_reader::{AsyncHttpRangeReader, CheckSupportMethod};
use async_zip::base::read::seek::ZipFileReader;
use clap::Parser;
use color_eyre::eyre::{ContextCompat, Error, Result};
use reqwest::header::HeaderMap;
use tokio::io::{AsyncRead, AsyncSeek};

use crate::pkg_name::{parse_dependency, PackageName, WheelFilename};
use crate::pypi::Package;

mod pkg_name;
mod pypi;

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
