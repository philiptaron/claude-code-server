//! The filesystem: a directory in which every version name is a symlink to a Claude Code.
//!
//! The directory lists as empty, so the server's pruning of old versions finds nothing to
//! delete, but looking up a version name always finds a symlink (see [`Catalog::resolve`]).
//! The mount is read-only; nothing can be created in it.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use fuser::{
    Errno, FileAttr, FileHandle, FileType, Filesystem, Generation, INodeNo, ReplyAttr, ReplyData,
    ReplyDirectory, ReplyEntry, Request,
};
use log::{debug, info};

use crate::builder::Builder;
use crate::catalog::{Catalog, Resolution};

/// How long the kernel may cache an answer. Short, so a version finished by the builder
/// replaces the stand-in quickly.
const TTL: Duration = Duration::from_secs(1);

pub struct CliFs {
    catalog: Catalog,
    builder: Option<Builder>,
    uid: u32,
    gid: u32,
    mounted_at: SystemTime,
    /// Inode numbers handed out to names, so a name keeps its inode across lookups.
    inodes: Mutex<Inodes>,
}

#[derive(Default)]
struct Inodes {
    by_name: HashMap<String, INodeNo>,
    names: Vec<String>,
}

impl Inodes {
    /// Inode numbers start after the root's.
    const FIRST: u64 = INodeNo::ROOT.0 + 1;

    fn get_or_insert(&mut self, name: &str) -> INodeNo {
        if let Some(ino) = self.by_name.get(name) {
            return *ino;
        }
        let ino = INodeNo(Self::FIRST + self.names.len() as u64);
        self.names.push(name.to_owned());
        self.by_name.insert(name.to_owned(), ino);
        ino
    }

    fn name(&self, ino: INodeNo) -> Option<&str> {
        let index = ino.0.checked_sub(Self::FIRST)?;
        self.names.get(usize::try_from(index).ok()?).map(String::as_str)
    }
}

impl CliFs {
    pub fn new(catalog: Catalog, builder: Option<Builder>) -> CliFs {
        // SAFETY: getuid and getgid cannot fail and touch no memory.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        CliFs {
            catalog,
            builder,
            uid,
            gid,
            mounted_at: SystemTime::now(),
            inodes: Mutex::default(),
        }
    }

    fn attr(&self, ino: INodeNo, kind: FileType, perm: u16, size: u64) -> FileAttr {
        FileAttr {
            ino,
            size,
            blocks: 0,
            atime: self.mounted_at,
            mtime: self.mounted_at,
            ctime: self.mounted_at,
            crtime: self.mounted_at,
            kind,
            perm,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        }
    }

    fn root_attr(&self) -> FileAttr {
        self.attr(INodeNo::ROOT, FileType::Directory, 0o755, 0)
    }

    fn link_attr(&self, ino: INodeNo, resolution: &Resolution) -> FileAttr {
        let size = resolution.target.as_os_str().len() as u64;
        self.attr(ino, FileType::Symlink, 0o777, size)
    }

    /// Resolve the name behind an inode afresh, so a finished build takes over at once.
    fn resolve_inode(&self, ino: INodeNo) -> Option<Resolution> {
        let name = self.inodes.lock().unwrap().name(ino)?.to_owned();
        self.catalog.resolve(&name)
    }
}

impl Filesystem for CliFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let resolution = match (parent, name.to_str()) {
            (INodeNo::ROOT, Some(name)) => self.catalog.resolve(name).map(|r| (name, r)),
            _ => None,
        };
        let Some((name, resolution)) = resolution else {
            debug!("lookup {name:?}: not a version, or no builds");
            reply.error(Errno::ENOENT);
            return;
        };
        if resolution.exact {
            info!("lookup {name} -> {}", resolution.target.display());
        } else {
            info!(
                "lookup {name} -> {} ({} standing in)",
                resolution.target.display(),
                resolution.version
            );
            if let Some(builder) = &self.builder {
                builder.request(name);
            }
        }
        let ino = self.inodes.lock().unwrap().get_or_insert(name);
        reply.entry(&TTL, &self.link_attr(ino, &resolution), Generation(0));
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        if ino == INodeNo::ROOT {
            reply.attr(&TTL, &self.root_attr());
        } else if let Some(resolution) = self.resolve_inode(ino) {
            reply.attr(&TTL, &self.link_attr(ino, &resolution));
        } else {
            reply.error(Errno::ENOENT);
        }
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        match self.resolve_inode(ino) {
            Some(resolution) => reply.data(resolution.target.as_os_str().as_bytes()),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        if ino != INodeNo::ROOT {
            reply.error(Errno::ENOTDIR);
            return;
        }
        let entries = [(INodeNo::ROOT, "."), (INodeNo::ROOT, "..")];
        for (i, (ino, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(ino, (i + 1) as u64, FileType::Directory, name) {
                break;
            }
        }
        reply.ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inodes_are_stable_per_name() {
        let mut inodes = Inodes::default();
        let a = inodes.get_or_insert("2.1.321");
        let b = inodes.get_or_insert("2.1.322");
        assert_ne!(a, b);
        assert_ne!(a, INodeNo::ROOT);
        assert_eq!(inodes.get_or_insert("2.1.321"), a);
        assert_eq!(inodes.name(a), Some("2.1.321"));
        assert_eq!(inodes.name(b), Some("2.1.322"));
        assert_eq!(inodes.name(INodeNo::ROOT), None);
        assert_eq!(inodes.name(INodeNo(1000)), None);
    }
}
