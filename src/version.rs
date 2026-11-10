//! Claude Code version names, as the desktop app's SSH server asks for them.
//!
//! The server looks up `<cli-dir>/<version>`, where the version is the bare release
//! (`2.1.322`), possibly with a suffix (`2.1.295-…`). Anything else (temporary files, the
//! `.zst` download, lock files) is not a version and must not be answered.

use std::cmp::Ordering;
use std::fmt;

/// The longest name accepted; real names are far shorter.
const MAX_LEN: usize = 128;

/// A parsed `MAJOR.MINOR.PATCH[-SUFFIX]` name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Version {
    numbers: [u64; 3],
    suffix: Option<String>,
}

impl Version {
    /// Parse a name matching `^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.+_-]+)?$`.
    pub fn parse(name: &str) -> Option<Version> {
        if name.is_empty() || name.len() > MAX_LEN {
            return None;
        }
        let (release, suffix) = match name.split_once('-') {
            Some((release, suffix)) => {
                let valid = !suffix.is_empty()
                    && suffix.bytes().all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b));
                if !valid {
                    return None;
                }
                (release, Some(suffix.to_owned()))
            }
            None => (name, None),
        };
        let mut numbers = [0u64; 3];
        let mut parts = release.split('.');
        for number in &mut numbers {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *number = part.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        Some(Version { numbers, suffix })
    }

    /// The same release without its suffix.
    pub fn release(&self) -> Version {
        Version { numbers: self.numbers, suffix: None }
    }

    pub fn has_suffix(&self) -> bool {
        self.suffix.is_some()
    }
}

impl Ord for Version {
    /// Numeric order of the release, then a bare release above any suffixed build of it (as
    /// semver orders pre-releases), then suffixes in byte order.
    fn cmp(&self, other: &Self) -> Ordering {
        self.numbers.cmp(&other.numbers).then_with(|| match (&self.suffix, &other.suffix) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.numbers;
        write!(f, "{major}.{minor}.{patch}")?;
        if let Some(suffix) = &self.suffix {
            write!(f, "-{suffix}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_versions() {
        for name in ["2.1.322", "0.0.0", "10.20.30", "2.1.295-abc", "2.1.295-a.b+c_d-e"] {
            let version = Version::parse(name).unwrap_or_else(|| panic!("{name} rejected"));
            assert_eq!(version.to_string(), name);
        }
    }

    #[test]
    fn rejects_other_names() {
        for name in [
            "",
            ".",
            "..",
            "2.1",
            "2.1.3.4",
            "2.1.322.zst",
            "2.1.322-",
            "2.1.322-a/b",
            "2.1.322-a b",
            ".2.1.322",
            "v2.1.322",
            "2..322",
            "2.1.x",
            "2.1.322.tmp",
            "+1.2.3",
            "99999999999999999999.0.0",
        ] {
            assert_eq!(Version::parse(name), None, "{name:?} accepted");
        }
        assert_eq!(Version::parse(&format!("1.2.3-{}", "a".repeat(200))), None);
    }

    #[test]
    fn orders_numerically() {
        let v = |name| Version::parse(name).unwrap();
        assert!(v("2.1.100") > v("2.1.99"));
        assert!(v("2.10.0") > v("2.9.99"));
        assert!(v("3.0.0") > v("2.99.99"));
        assert!(v("2.1.322") > v("2.1.322-abc"));
        assert!(v("2.1.322-abd") > v("2.1.322-abc"));
        assert!(v("2.1.323-abc") > v("2.1.322"));
    }

    #[test]
    fn release_drops_suffix() {
        let v = Version::parse("2.1.295-abc").unwrap();
        assert!(v.has_suffix());
        assert_eq!(v.release(), Version::parse("2.1.295").unwrap());
    }
}
