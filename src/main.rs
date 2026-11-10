//! ccd-cli-fs mounts a FUSE filesystem at the Claude desktop app's SSH remote CLI directory
//! (`~/.claude/remote/ccd-cli`) in which every version name is a symlink to a Nix-built
//! Claude Code, so the app's generic glibc download, which NixOS cannot run, is never needed.

mod builder;
mod catalog;
mod fs;
mod version;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;
use std::{env, io, mem, ptr, thread};

use clap::Parser;
use fuser::{Config, MountOption};
use log::{error, info, warn};

use crate::builder::Builder;
use crate::catalog::Catalog;
use crate::fs::CliFs;
use crate::version::Version;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Where to mount, usually ~/.claude/remote/ccd-cli
    mountpoint: PathBuf,

    /// A Claude Code build to serve: VERSION=PATH, where PATH is the executable
    #[arg(long = "cli", value_name = "VERSION=PATH", value_parser = parse_cli)]
    clis: Vec<(Version, PathBuf)>,

    /// A directory of builds named by version: out-links to packages with bin/claude, or
    /// executables. Read again on every lookup
    #[arg(long, value_name = "DIR")]
    versions_dir: Option<PathBuf>,

    /// A command run as `BUILDER VERSION` in the background when a version is missing; it
    /// should leave the build in the versions directory
    #[arg(long, value_name = "BUILDER", requires = "versions_dir")]
    builder: Option<PathBuf>,

    /// How long to wait before building a version again after its build failed
    #[arg(long, value_name = "SECONDS", default_value_t = 600)]
    retry_after: u64,
}

fn parse_cli(arg: &str) -> Result<(Version, PathBuf), String> {
    let (version, path) = arg.split_once('=').ok_or("expected VERSION=PATH")?;
    let version = Version::parse(version).ok_or_else(|| format!("{version:?} is not a version"))?;
    if path.is_empty() {
        return Err("empty path".into());
    }
    Ok((version, PathBuf::from(path)))
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,fuser=error"))
        .format_timestamp(None)
        .init();
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> io::Result<()> {
    let mut clis = BTreeMap::new();
    for (version, path) in args.clis {
        if !path.exists() {
            warn!("{version}: {} does not exist", path.display());
        }
        clis.insert(version, path);
    }
    let catalog = Catalog::new(clis, args.versions_dir);
    let available = catalog.available();
    match available.keys().next_back() {
        Some(newest) => info!("{} builds, newest {newest}", available.len()),
        None => warn!("no builds: every lookup will fail until one appears"),
    }
    let builder = args.builder.map(|b| Builder::new(b, Duration::from_secs(args.retry_after)));

    // Signals are taken by a thread of their own (see `unmount_on_signal`), so they must be
    // blocked before any other thread starts, and so inherits the mask.
    let signals = block_signals()?;
    let mountpoint = args.mountpoint.clone();
    thread::spawn(move || unmount_on_signal(signals, mountpoint));

    let mut config = Config::default();
    config.mount_options.extend([
        MountOption::FSName("ccd-cli-fs".into()),
        MountOption::Subtype("ccd-cli-fs".into()),
        MountOption::RO,
        MountOption::DefaultPermissions,
        MountOption::NoDev,
        MountOption::NoSuid,
    ]);
    info!("mounting at {}", args.mountpoint.display());
    fuser::mount(CliFs::new(catalog, builder), &args.mountpoint, &config)?;
    info!("unmounted");
    Ok(())
}

const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

fn block_signals() -> io::Result<libc::sigset_t> {
    // SAFETY: the set is initialized by sigemptyset before use, and pthread_sigmask only
    // reads it.
    unsafe {
        let mut set: libc::sigset_t = mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in SIGNALS {
            libc::sigaddset(&mut set, signal);
        }
        match libc::pthread_sigmask(libc::SIG_BLOCK, &set, ptr::null_mut()) {
            0 => Ok(set),
            errno => Err(io::Error::from_raw_os_error(errno)),
        }
    }
}

/// Wait for a termination signal, then unmount, which ends `fuser::mount` in the main
/// thread. Unmounting needs `fusermount3`, found the way fuser finds it for mounting.
fn unmount_on_signal(signals: libc::sigset_t, mountpoint: PathBuf) {
    let mut signal = 0;
    // SAFETY: the set was initialized by block_signals, and sigwait writes only `signal`.
    unsafe { libc::sigwait(&signals, &mut signal) };
    info!("signal {signal}, unmounting");
    let fusermount =
        env::var_os("FUSERMOUNT_PATH").unwrap_or_else(|| OsString::from("fusermount3"));
    let unmounted = Command::new(&fusermount)
        .args(["-u", "-z", "--"])
        .arg(&mountpoint)
        .status()
        .is_ok_and(|status| status.success());
    if !unmounted {
        error!("could not unmount {} with {fusermount:?}", mountpoint.display());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_arguments() {
        let (version, path) = parse_cli("2.1.322=/nix/store/x/bin/claude").unwrap();
        assert_eq!(
            (version.to_string(), path),
            ("2.1.322".into(), "/nix/store/x/bin/claude".into())
        );
        assert!(parse_cli("2.1.322").is_err());
        assert!(parse_cli("2.1.322=").is_err());
        assert!(parse_cli("latest=/x").is_err());
    }

    #[test]
    fn arguments_parse() {
        use clap::CommandFactory;
        Args::command().debug_assert();
    }
}
