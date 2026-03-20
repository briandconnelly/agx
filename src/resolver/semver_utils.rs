//! Semver constraint parsing and version resolution.

use thiserror::Error;

/// Errors from version resolution.
#[derive(Debug, Error)]
pub enum ResolverError {
    /// No available version satisfies the given constraint.
    #[error("no version of '{package}' satisfies constraint '{constraint}'")]
    NoMatch {
        package: String,
        constraint: String,
    },

    /// Multiple plugins provide the same-named asset without include/exclude resolving it.
    #[error("conflict: asset '{name}' is provided by multiple plugins: {sources:?}")]
    Conflict {
        name: String,
        sources: Vec<String>,
    },

    /// A version constraint string could not be parsed.
    #[error("invalid version constraint: {0}")]
    InvalidConstraint(String),

    /// A named dependency was not found in the manifest.
    #[error("dependency '{name}' not found in manifest")]
    NotFound {
        name: String,
    },

    /// A referenced registry was not found in the resolve context.
    #[error("registry '{registry}' not found in resolve context")]
    RegistryNotFound {
        registry: String,
    },

    /// A git ref could not be resolved to a commit SHA.
    #[error("git ref '{git_ref}' for '{url}' was not pre-resolved")]
    GitRefNotResolved {
        url: String,
        git_ref: String,
    },

    /// An invalid source configuration for a dependency.
    #[error("invalid source for '{name}': {reason}")]
    InvalidSource {
        name: String,
        reason: String,
    },
}

/// Parse a version constraint string into a semver::VersionReq.
pub fn parse_constraint(constraint: &str) -> Result<semver::VersionReq, ResolverError> {
    semver::VersionReq::parse(constraint)
        .map_err(|_| ResolverError::InvalidConstraint(constraint.to_string()))
}

/// Check if a version satisfies a constraint.
pub fn satisfies(version: &semver::Version, constraint: &semver::VersionReq) -> bool {
    constraint.matches(version)
}

/// Given a list of available versions, find the best match for a constraint.
pub fn resolve_best(
    available: &[semver::Version],
    constraint: &semver::VersionReq,
) -> Option<semver::Version> {
    available
        .iter()
        .filter(|v| constraint.matches(v))
        .max()
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_version() {
        let req = parse_constraint("=1.2.0").unwrap();
        let v = semver::Version::new(1, 2, 0);
        assert!(satisfies(&v, &req));
        assert!(!satisfies(&semver::Version::new(1, 2, 1), &req));
    }

    #[test]
    fn range_constraint() {
        let req = parse_constraint(">=1.0.0, <2.0.0").unwrap();
        assert!(satisfies(&semver::Version::new(1, 0, 0), &req));
        assert!(satisfies(&semver::Version::new(1, 5, 3), &req));
        assert!(!satisfies(&semver::Version::new(2, 0, 0), &req));
        assert!(!satisfies(&semver::Version::new(0, 9, 9), &req));
    }

    #[test]
    fn tilde_constraint() {
        let req = parse_constraint("~1.3.0").unwrap();
        assert!(satisfies(&semver::Version::new(1, 3, 0), &req));
        assert!(satisfies(&semver::Version::new(1, 3, 2), &req));
        assert!(!satisfies(&semver::Version::new(1, 4, 0), &req));
        assert!(!satisfies(&semver::Version::new(1, 2, 9), &req));
    }

    #[test]
    fn tilde_constraint_minor_only() {
        // ~1.3 should match >=1.3.0, <1.4.0
        let req = parse_constraint("~1.3").unwrap();
        assert!(satisfies(&semver::Version::new(1, 3, 0), &req));
        assert!(satisfies(&semver::Version::new(1, 3, 99), &req));
        assert!(!satisfies(&semver::Version::new(1, 4, 0), &req));
    }

    #[test]
    fn caret_constraint() {
        let req = parse_constraint("^1.3.0").unwrap();
        assert!(satisfies(&semver::Version::new(1, 3, 0), &req));
        assert!(satisfies(&semver::Version::new(1, 9, 0), &req));
        assert!(!satisfies(&semver::Version::new(2, 0, 0), &req));
        assert!(!satisfies(&semver::Version::new(1, 2, 9), &req));
    }

    #[test]
    fn caret_constraint_zero_major() {
        // ^0.2.3 should match >=0.2.3, <0.3.0
        let req = parse_constraint("^0.2.3").unwrap();
        assert!(satisfies(&semver::Version::new(0, 2, 3), &req));
        assert!(satisfies(&semver::Version::new(0, 2, 9), &req));
        assert!(!satisfies(&semver::Version::new(0, 3, 0), &req));
        assert!(!satisfies(&semver::Version::new(1, 0, 0), &req));
    }

    #[test]
    fn caret_constraint_zero_minor() {
        // ^0.0.3 should match >=0.0.3, <0.0.4
        let req = parse_constraint("^0.0.3").unwrap();
        assert!(satisfies(&semver::Version::new(0, 0, 3), &req));
        assert!(!satisfies(&semver::Version::new(0, 0, 4), &req));
    }

    #[test]
    fn resolve_best_version() {
        let versions = vec![
            semver::Version::new(1, 0, 0),
            semver::Version::new(1, 2, 0),
            semver::Version::new(1, 3, 0),
            semver::Version::new(2, 0, 0),
        ];
        let req = parse_constraint("^1.0.0").unwrap();
        let best = resolve_best(&versions, &req).unwrap();
        assert_eq!(best, semver::Version::new(1, 3, 0));
    }

    #[test]
    fn resolve_best_with_range() {
        let versions = vec![
            semver::Version::new(1, 0, 0),
            semver::Version::new(1, 5, 0),
            semver::Version::new(1, 9, 0),
            semver::Version::new(2, 0, 0),
        ];
        let req = parse_constraint(">=1.0.0, <2.0.0").unwrap();
        let best = resolve_best(&versions, &req).unwrap();
        assert_eq!(best, semver::Version::new(1, 9, 0));
    }

    #[test]
    fn resolve_best_no_match() {
        let versions = vec![
            semver::Version::new(2, 0, 0),
            semver::Version::new(3, 0, 0),
        ];
        let req = parse_constraint("^1.0.0").unwrap();
        assert!(resolve_best(&versions, &req).is_none());
    }

    #[test]
    fn resolve_best_empty_versions() {
        let versions: Vec<semver::Version> = vec![];
        let req = parse_constraint("^1.0.0").unwrap();
        assert!(resolve_best(&versions, &req).is_none());
    }

    #[test]
    fn invalid_constraint() {
        assert!(parse_constraint("not a version").is_err());
    }

    #[test]
    fn prerelease_not_matched_by_default() {
        let req = parse_constraint("^1.0.0").unwrap();
        let pre = semver::Version::parse("1.2.0-alpha").unwrap();
        // semver crate: pre-releases are not matched by default
        assert!(!satisfies(&pre, &req));
    }

    #[test]
    fn wildcard_constraint() {
        let req = parse_constraint("*").unwrap();
        assert!(satisfies(&semver::Version::new(0, 0, 1), &req));
        assert!(satisfies(&semver::Version::new(99, 99, 99), &req));
    }
}
