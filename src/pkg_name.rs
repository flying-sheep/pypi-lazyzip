use std::{fmt::Display, str::FromStr, sync::LazyLock};

use caseless::Caseless;
use color_eyre::eyre::{bail, Context as _, Error, OptionExt as _, Result};
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
static ID_START_RE: LazyLock<regex::Regex> =
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

/// A wheel filename, partially parsed.
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
        // `tags` will contain 3-4 `-`s (build tag is optional)
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

pub fn parse_dependency(s: &str) -> Result<(PackageName, Option<pep440_rs::VersionSpecifier>)> {
    let Some(name) = ID_START_RE.find(s) else {
        bail!("invalid identifier");
    };
    let rest = &s[name.len()..];
    let version_spec = (!rest.is_empty())
        .then(|| pep440_rs::VersionSpecifier::from_str(rest))
        .transpose()
        .with_context(|| format!("could not parse version from {rest}"))?;
    Ok((PackageName::from_str(name.as_str())?, version_spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str() {
        assert!(matches!(parse_dependency("foo"), Ok((_, None))));
        assert!(matches!(parse_dependency("foo==1.0"), Ok((_, Some(_)))));
        assert!(matches!(parse_dependency("foo ==1.0.1"), Ok((_, Some(_)))));
        assert!(parse_dependency("foo!!1.0").is_err());
        assert!(parse_dependency("-_==1.0").is_err());
    }
}
