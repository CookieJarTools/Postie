//! Notices when a newer Postie replaces the copy the service started from, as The Jar does when it
//! updates it, so the service can restart into the new version instead of running the old one until
//! the next login.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub struct Program {
    /// The AppImage when running from one, otherwise the binary itself.
    path: PathBuf,
    appimage: bool,
    started_as: Option<Identity>,
}

/// Enough to tell a file that was swapped or rewritten from the one that was there before.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Identity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

fn identity(path: &Path) -> Option<Identity> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Identity {
        dev: meta.dev(),
        ino: meta.ino(),
        len: meta.len(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
    })
}

impl Program {
    pub fn current() -> Self {
        let (path, appimage) = match std::env::var_os("APPIMAGE") {
            Some(appimage) => (PathBuf::from(appimage), true),
            None => (std::env::current_exe().unwrap_or_default(), false),
        };
        let started_as = identity(&path);
        Self { path, appimage, started_as }
    }

    /// The file the service started from: the AppImage, or the binary.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether there's a newer copy to restart into: a different file where this one was, or for an
    /// AppImage that's gone, another AppImage beside it, which is what the launcher starts. A copy
    /// that was deleted outright, as when Postie is uninstalled, isn't a reason to restart.
    pub fn replaced(&self) -> bool {
        let Some(started_as) = self.started_as else { return false };
        match identity(&self.path) {
            Some(now) => now != started_as,
            None => self.appimage && self.path.parent().is_some_and(has_appimage),
        }
    }
}

fn has_appimage(folder: &Path) -> bool {
    std::fs::read_dir(folder).is_ok_and(|entries| {
        entries.flatten().any(|entry| entry.path().extension().is_some_and(|ext| ext == "AppImage"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty folder of its own under the system's temporary folder.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("postie-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn program(path: &Path, appimage: bool) -> Program {
        Program { path: path.to_path_buf(), appimage, started_as: identity(path) }
    }

    #[test]
    fn notices_a_file_swapped_in_place() {
        let dir = scratch("swapped");
        let file = dir.join("postie");
        std::fs::write(&file, "old").unwrap();
        let program = program(&file, false);
        assert!(!program.replaced());

        let new = dir.join("postie.new");
        std::fs::write(&new, "new").unwrap();
        std::fs::rename(&new, &file).unwrap();
        assert!(program.replaced());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_appimage_replaced_by_a_newer_one_beside_it_counts_but_a_deleted_one_doesnt() {
        let dir = scratch("beside");
        let old = dir.join("Postie-0.1.1.AppImage");
        std::fs::write(&old, "old").unwrap();
        let appimage = program(&old, true);
        let binary = program(&old, false);

        std::fs::remove_file(&old).unwrap();
        assert!(!appimage.replaced());
        assert!(!binary.replaced());

        std::fs::write(dir.join("Postie-0.1.2.AppImage"), "new").unwrap();
        assert!(appimage.replaced());
        assert!(!binary.replaced());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
