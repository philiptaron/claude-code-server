//! Background builds of versions that are not in the catalog.
//!
//! A lookup never waits for one: the server's probe has a time limit, so the newest build
//! answers in the meantime. Each build is an instance of a systemd template unit, started
//! as `systemctl --user start --no-block ccd-cli-fs-build@VERSION.service`; on success it
//! leaves an entry for the version in the versions directory, which the catalog picks up on
//! the next lookup.
//!
//! Nothing is tracked here. systemd runs one instance per version however often it is
//! asked, and the unit's start limit keeps a build that failed, or that left nothing to
//! serve, from running again on every lookup.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use log::{info, warn};

pub struct Builder {
    systemctl: PathBuf,
    /// The unit name up to and including `@`, and from `.` on.
    prefix: String,
    suffix: String,
}

impl Builder {
    /// `template` is a template unit's name, such as `ccd-cli-fs-build@.service`. `None`
    /// if it is not one.
    pub fn new(systemctl: PathBuf, template: &str) -> Option<Builder> {
        let at = template.find("@.")?;
        let (prefix, suffix) = template.split_at(at + 1);
        (at > 0 && suffix.len() > 1).then(|| Builder {
            systemctl,
            prefix: prefix.to_owned(),
            suffix: suffix.to_owned(),
        })
    }

    /// The instance of the template that builds `version`. A release is digits and dots,
    /// which need no escaping in a unit name.
    fn unit(&self, version: &str) -> String {
        format!("{}{version}{}", self.prefix, self.suffix)
    }

    /// Ask systemd to start the build of `version`, without waiting for systemd's answer:
    /// the lookup that asks must not be held up.
    pub fn request(&self, version: &str) {
        let unit = self.unit(version);
        let systemctl = self.systemctl.clone();
        thread::spawn(move || {
            let status = Command::new(&systemctl)
                .args(["--user", "start", "--no-block"])
                .arg(&unit)
                .stdin(Stdio::null())
                .status();
            match status {
                Ok(status) if status.success() => info!("asked for {unit}"),
                Ok(status) => warn!("starting {unit} failed: {status}"),
                Err(error) => warn!("could not run {}: {error}", systemctl.display()),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names() {
        let builder = Builder::new("systemctl".into(), "ccd-cli-fs-build@.service").unwrap();
        assert_eq!(builder.unit("2.1.322"), "ccd-cli-fs-build@2.1.322.service");
    }

    #[test]
    fn rejects_what_is_not_a_template() {
        for name in ["ccd-cli-fs-build.service", "ccd-cli-fs-build@", "@.service", "build@."] {
            assert!(Builder::new("systemctl".into(), name).is_none(), "{name:?} accepted");
        }
    }
}
