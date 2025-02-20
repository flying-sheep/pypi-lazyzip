use std::{fmt::Display, str::FromStr};

use color_eyre::eyre::{Context as _, Error, bail};

use super::package_name::{ID_START_RE, PackageName};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dependency {
    name: PackageName,
    version_spec: Option<pep440_rs::VersionSpecifier>,
}

impl Dependency {
    pub fn new(name: PackageName, version_spec: Option<pep440_rs::VersionSpecifier>) -> Self {
        Self { name, version_spec }
    }

    #[allow(dead_code)]
    pub fn has_version_spec(&self) -> bool {
        self.version_spec.is_some()
    }

    pub fn name(&self) -> &PackageName {
        &self.name
    }

    pub fn into_name(self) -> PackageName {
        self.name
    }

    pub fn version_spec(&self) -> Option<&pep440_rs::VersionSpecifier> {
        self.version_spec.as_ref()
    }

    #[allow(dead_code)]
    pub fn into_version_spec(self) -> Option<pep440_rs::VersionSpecifier> {
        self.version_spec
    }

    #[allow(dead_code)]
    pub fn into_inner(self) -> (PackageName, Option<pep440_rs::VersionSpecifier>) {
        (self.name, self.version_spec)
    }
}

impl FromStr for Dependency {
    type Err = Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let Some(name) = ID_START_RE.find(s) else {
            bail!("invalid identifier");
        };
        let rest = &s[name.len()..];
        let version_spec = (!rest.is_empty())
            .then(|| pep440_rs::VersionSpecifier::from_str(rest))
            .transpose()
            .with_context(|| format!("could not parse version from {rest}"))?;
        Ok(Self::new(
            PackageName::from_str(name.as_str())?,
            version_spec,
        ))
    }
}

impl Display for Dependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.name.fmt(f)?;
        self.version_spec().map(|vs| vs.fmt(f)).transpose()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_str() {
        assert!(Dependency::from_str("foo").is_ok_and(|v| !v.has_version_spec()));
        assert!(Dependency::from_str("foo==1.0").is_ok_and(|v| v.has_version_spec()));
        assert!(Dependency::from_str("foo ==1.0.1").is_ok_and(|v| v.has_version_spec()));
        assert!(Dependency::from_str("foo!!1.0").is_err());
        assert!(Dependency::from_str("-_==1.0").is_err());
    }
}
