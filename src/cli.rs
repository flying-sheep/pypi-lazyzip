use std::{path::PathBuf, str::FromStr};

use color_eyre::eyre::Error;

use crate::python_pkg::Dependency;

#[derive(Debug, Clone)]
pub enum PkgLoc {
    Dependency(Dependency),
    Path(PathBuf),
}

impl std::fmt::Display for PkgLoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgLoc::Dependency(dep) => {
                dep.name().fmt(f)?;
                dep.version_spec().map(|vs| vs.fmt(f)).transpose()?;
                Ok(())
            }
            PkgLoc::Path(path) => path.display().fmt(f),
        }
    }
}

impl FromStr for PkgLoc {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(dep) = Dependency::from_str(s) {
            Ok(PkgLoc::Dependency(dep))
        } else {
            Ok(PkgLoc::Path(PathBuf::from(s)))
        }
    }
}

#[derive(clap::Parser)]
pub struct Cli {
    pub pkg_locs: Vec<PkgLoc>,
}
