//! The Claude Code builds the filesystem can hand out, and which one answers a name.
//!
//! Builds come from two places: the ones fixed when the daemon was built (`--cli
//! VERSION=PATH`, store paths that Nix keeps alive), and a directory of out-links
//! (`--versions-dir`) that the background builder, or a person, fills at runtime. The
//! directory is read again on every resolution, so a finished build is served at once.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::version::Version;

/// How a name was answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    /// The executable the name's symlink points at.
    pub target: PathBuf,
    /// The version of that executable.
    pub version: Version,
    /// Whether that is the version asked for (or its release, for a suffixed name), rather
    /// than the newest build standing in for a version that is not here.
    pub exact: bool,
}

pub struct Catalog {
    fixed: BTreeMap<Version, PathBuf>,
    versions_dir: Option<PathBuf>,
}

impl Catalog {
    pub fn new(fixed: BTreeMap<Version, PathBuf>, versions_dir: Option<PathBuf>) -> Catalog {
        Catalog { fixed, versions_dir }
    }

    /// Every build available now, fixed builds taking precedence over the directory.
    pub fn available(&self) -> BTreeMap<Version, PathBuf> {
        let mut builds = self.versions_dir.as_deref().map(scan_versions_dir).unwrap_or_default();
        builds.extend(self.fixed.iter().map(|(v, p)| (v.clone(), p.clone())));
        builds
    }

    /// Answer a looked-up name: the build of that version, else the build of its release
    /// (for a suffixed name), else the newest build. `None` when the name is not a version
    /// or there are no builds at all.
    pub fn resolve(&self, name: &str) -> Option<Resolution> {
        let wanted = Version::parse(name)?;
        let builds = self.available();
        let found = |version: &Version| {
            builds.get(version).map(|target| Resolution {
                target: target.clone(),
                version: version.clone(),
                exact: true,
            })
        };
        found(&wanted)
            .or_else(|| wanted.has_suffix().then(|| found(&wanted.release())).flatten())
            .or_else(|| {
                builds.iter().next_back().map(|(version, target)| Resolution {
                    target: target.clone(),
                    version: version.clone(),
                    exact: false,
                })
            })
    }
}

/// The builds in a directory of entries named by version: each an out-link to a package
/// with `bin/claude`, or an executable itself. Targets are canonicalized, so the symlinks
/// the filesystem serves point into the store rather than back into the directory.
fn scan_versions_dir(dir: &Path) -> BTreeMap<Version, PathBuf> {
    let mut builds = BTreeMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return builds;
    };
    for entry in entries.flatten() {
        let Some(version) = entry.file_name().to_str().and_then(Version::parse) else {
            continue;
        };
        let path = entry.path();
        let candidates = [path.join("bin/claude"), path];
        if let Some(target) = candidates.iter().find_map(|c| executable(c)) {
            builds.insert(version, target);
        }
    }
    builds
}

/// The canonical path of `path` if it is an executable regular file that starts as an ELF
/// binary or a script does. The server's probe reads the start of the file and rejects
/// anything else, such as the empty file a failed download leaves, without running it.
fn executable(path: &Path) -> Option<PathBuf> {
    let target = fs::canonicalize(path).ok()?;
    let mut file = fs::File::open(&target).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return None;
    }
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    (magic == *b"\x7fELF" || magic.starts_with(b"#!")).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn v(name: &str) -> Version {
        Version::parse(name).unwrap()
    }

    fn fixed(entries: &[(&str, &str)]) -> BTreeMap<Version, PathBuf> {
        entries.iter().map(|(name, path)| (v(name), PathBuf::from(path))).collect()
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ccd-cli-fs-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn exact_match() {
        let catalog = Catalog::new(fixed(&[("2.1.321", "/a"), ("2.1.322", "/b")]), None);
        let r = catalog.resolve("2.1.321").unwrap();
        assert_eq!((r.target, r.version, r.exact), (PathBuf::from("/a"), v("2.1.321"), true));
    }

    #[test]
    fn unknown_falls_back_to_newest() {
        let catalog = Catalog::new(fixed(&[("2.1.99", "/a"), ("2.1.100", "/b")]), None);
        for name in ["2.1.50", "9.9.9", "2.1.101-abc"] {
            let r = catalog.resolve(name).unwrap();
            assert_eq!((r.target, r.version, r.exact), (PathBuf::from("/b"), v("2.1.100"), false));
        }
    }

    #[test]
    fn suffixed_name_uses_its_release() {
        let catalog = Catalog::new(fixed(&[("2.1.295", "/a"), ("2.1.322", "/b")]), None);
        let r = catalog.resolve("2.1.295-deadbeef").unwrap();
        assert_eq!((r.target, r.exact), (PathBuf::from("/a"), true));
    }

    #[test]
    fn non_versions_and_empty_catalog() {
        let catalog = Catalog::new(fixed(&[("2.1.322", "/b")]), None);
        assert_eq!(catalog.resolve("2.1.322.zst"), None);
        assert_eq!(catalog.resolve(".lock"), None);
        assert_eq!(Catalog::new(BTreeMap::new(), None).resolve("2.1.322"), None);
    }

    #[test]
    fn versions_dir_entries() {
        let dir = scratch();
        // an out-link to a package
        let package = dir.join("store/claude-code-2.1.300");
        write_executable(&package.join("bin/claude"));
        symlink(&package, dir.join("2.1.300")).unwrap();
        // a bare executable
        write_executable(&dir.join("2.1.301"));
        // not executable, empty, not a binary or script, not a version, dangling
        fs::write(dir.join("2.1.302"), "#!/bin/sh\n").unwrap();
        fs::write(dir.join("2.1.304"), "").unwrap();
        fs::set_permissions(dir.join("2.1.304"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.join("2.1.305"), "garbage").unwrap();
        fs::set_permissions(dir.join("2.1.305"), fs::Permissions::from_mode(0o755)).unwrap();
        write_executable(&dir.join("notes"));
        symlink(dir.join("missing"), dir.join("2.1.303")).unwrap();

        let catalog = Catalog::new(fixed(&[("2.1.200", "/fixed")]), Some(dir.clone()));
        let available = catalog.available();
        assert_eq!(
            available.keys().cloned().collect::<Vec<_>>(),
            [v("2.1.200"), v("2.1.300"), v("2.1.301")]
        );
        let canonical = fs::canonicalize(package.join("bin/claude")).unwrap();
        assert_eq!(catalog.resolve("2.1.300").unwrap().target, canonical);
        // the newest, from the directory, stands in for unknown versions
        assert_eq!(catalog.resolve("2.1.999").unwrap().version, v("2.1.301"));
        // a fixed build wins over the directory
        write_executable(&dir.join("2.1.200"));
        assert_eq!(catalog.resolve("2.1.200").unwrap().target, PathBuf::from("/fixed"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_versions_dir() {
        let catalog = Catalog::new(fixed(&[("2.1.200", "/fixed")]), Some("/nonexistent".into()));
        assert_eq!(catalog.resolve("2.1.200").unwrap().target, PathBuf::from("/fixed"));
    }
}
