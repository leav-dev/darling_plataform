//! Numeric comparison of dotted versions.
//!
//! macOS and Darwin versions, and the `LSMinimumSystemVersion` values bundles
//! declare, are dotted numeric strings. Comparing them as text puts `10.9` above
//! `10.15`, so the profile and the analysis compare numeric components instead,
//! with missing trailing components read as zero (`11` and `11.0` are the same
//! requirement).
//!
//! This module is private: it is a comparison rule, not part of the report.

use std::cmp::Ordering;

/// A version made of numeric, dot-separated components.
///
/// Equality is ordinal equality: `11` and `11.0` are the same requirement, so a
/// derived `PartialEq` over the stored components would contradict [`Ord`].
#[derive(Debug, Clone)]
pub(crate) struct Version {
    components: Vec<u64>,
}

impl Version {
    /// Parses `text`, or reports `None` when any component is not a number.
    ///
    /// Surrounding whitespace is ignored, and an empty component (`11..1`), an
    /// empty string or a negative component is rejected: a version the engine
    /// cannot order is reported as unknown, never guessed.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut components = Vec::new();
        for part in text.split('.') {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            components.push(part.parse::<u64>().ok()?);
        }
        Some(Self { components })
    }

    /// Component at `index`, or zero when the version stops before it.
    fn component(&self, index: usize) -> u64 {
        self.components.get(index).copied().unwrap_or(0)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let width = self.components.len().max(other.components.len());
        for index in 0..width {
            match self.component(index).cmp(&other.component(index)) {
                Ordering::Equal => {}
                ordering => return ordering,
            }
        }
        Ordering::Equal
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("'{text}' is a version"))
    }

    #[test]
    fn compares_components_numerically_not_as_text() {
        assert!(version("10.9") < version("10.15"));
        assert!(version("9.9.9") < version("10.0"));
        assert!(version("11.0.1") > version("11.0"));
    }

    #[test]
    fn reads_missing_trailing_components_as_zero() {
        assert_eq!(version("11"), version("11.0"));
        assert_eq!(version("11.0"), version("11.0.0"));
        assert!(version("11.0.0") <= version("11.0"));
    }

    #[test]
    fn ignores_surrounding_whitespace() {
        assert_eq!(version(" 20.6.0 "), version("20.6.0"));
    }

    #[test]
    fn rejects_values_that_are_not_numeric_components() {
        for text in [
            "",
            " ",
            "Big Sur",
            "11.",
            ".11",
            "11..0",
            "11.0-beta",
            "-1.0",
        ] {
            assert!(
                Version::parse(text).is_none(),
                "'{text}' must not parse as a version"
            );
        }
    }
}
