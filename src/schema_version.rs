use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Represents a schema version with semantic versioning
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SchemaVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl FromStr for SchemaVersion {
    type Err = SchemaVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(SchemaVersionError::InvalidFormat(s.to_string()));
        }

        let major = parts[0].parse().map_err(|_| SchemaVersionError::InvalidFormat(s.to_string()))?;
        let minor = parts[1].parse().map_err(|_| SchemaVersionError::InvalidFormat(s.to_string()))?;
        let patch = parts[2].parse().map_err(|_| SchemaVersionError::InvalidFormat(s.to_string()))?;

        Ok(SchemaVersion::new(major, minor, patch))
    }
}

impl PartialOrd for SchemaVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SchemaVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major.cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
    }
}

#[derive(Debug, Error)]
pub enum SchemaVersionError {
    #[error("Invalid version format: {0}")]
    InvalidFormat(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_version_creation() {
        let version = SchemaVersion::new(5, 0, 10);
        assert_eq!(version.major, 5);
        assert_eq!(version.minor, 0);
        assert_eq!(version.patch, 10);
    }

    #[test]
    fn test_schema_version_display() {
        let version = SchemaVersion::new(25, 2, 9);
        assert_eq!(version.to_string(), "25.2.9");
    }

    #[test]
    fn test_schema_version_from_str() {
        let version: SchemaVersion = "23.2.24".parse().unwrap();
        assert_eq!(version, SchemaVersion::new(23, 2, 24));
    }

    #[test]
    fn test_schema_version_ordering() {
        let v1 = SchemaVersion::new(5, 0, 10);
        let v2 = SchemaVersion::new(23, 2, 24);
        let v3 = SchemaVersion::new(25, 2, 9);

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v1 < v3);
    }

    #[test]
    fn test_invalid_version_format() {
        assert!("invalid".parse::<SchemaVersion>().is_err());
        assert!("1.2".parse::<SchemaVersion>().is_err());
        assert!("1.2.3.4".parse::<SchemaVersion>().is_err());
    }
}