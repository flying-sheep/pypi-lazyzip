use std::{fmt::Display, str::FromStr, sync::LazyLock};

use caseless::Caseless;
use color_eyre::eyre::{Error, bail};
use serde::Serialize;

/// A Python package name, normalized for comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName(String);

impl Serialize for PackageName {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

/// Regex matching strings that start with valid Python package identifiers
pub(super) static ID_START_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?i)^[A-Z0-9][A-Z0-9._-]*[A-Z0-9]|[A-Z0-9]").unwrap());

impl FromStr for PackageName {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // If the match is not as long as the whole thing, there is more after
        if ID_START_RE.find(s).is_none_or(|m| m.len() != s.len()) {
            bail!("invalid identifier");
        }
        Ok(PackageName(
            s.chars()
                .default_case_fold()
                .map(|c| if c == '_' { '-' } else { c })
                .collect(),
        ))
    }
}

impl From<PackageName> for String {
    fn from(value: PackageName) -> Self {
        value.0
    }
}

impl Display for PackageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
