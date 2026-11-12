//! Publish a finished staging file under a new name without ever replacing an
//! existing file.
//!
//! A hard link is the preferred way: it is atomic and fails if the name exists.
//! exFAT and FAT volumes (most USB sticks and SD cards) and some network shares
//! have no hard links, so there the destination is claimed with an exclusive
//! create and the bytes are copied. That path still never replaces a file; if
//! the copy fails, only the file this call created is removed.
use std::{
    fs::{self, OpenOptions},
    io,
    path::Path,
};

/// Create `destination` with the contents of `staged`. Fails with
/// `AlreadyExists` when the name is taken. The caller keeps and removes `staged`.
pub fn publish_new(staged: &Path, destination: &Path) -> io::Result<()> {
    publish_new_with(|from, to| fs::hard_link(from, to), staged, destination)
}

pub(crate) fn publish_new_with(
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
    staged: &Path,
    destination: &Path,
) -> io::Result<()> {
    match link(staged, destination) {
        Ok(()) => return Ok(()),
        // The name is taken, or the staging file is gone: copying cannot help.
        Err(e) if matches!(e.kind(), io::ErrorKind::AlreadyExists | io::ErrorKind::NotFound) => return Err(e),
        // Anything else may be a volume without hard links. A real permission
        // or I/O problem fails the exclusive create below the same way.
        Err(_) => {}
    }
    let mut source = fs::File::open(staged)?;
    // Ownership is established here: an existing file or symlink is never opened.
    let mut target = OpenOptions::new().write(true).create_new(true).open(destination)?;
    let copied = io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
    drop(target);
    if let Err(e) = copied {
        let _ = fs::remove_file(destination);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_links(_: &Path, _: &Path) -> io::Result<()> {
        // What exFAT reports on Linux; macOS and Windows report "unsupported".
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    #[test]
    fn a_volume_without_hard_links_still_receives_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (staged, destination) = (dir.path().join("staged"), dir.path().join("final"));
        fs::write(&staged, b"catalog bytes").unwrap();
        publish_new_with(no_links, &staged, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"catalog bytes");
        assert!(staged.exists(), "the caller removes its own staging file");
        publish_new_with(|_, _| Err(io::Error::from(io::ErrorKind::Unsupported)), &staged, &dir.path().join("second")).unwrap();
    }

    #[test]
    fn the_copy_path_never_replaces_an_existing_file_or_follows_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let (staged, destination) = (dir.path().join("staged"), dir.path().join("final"));
        fs::write(&staged, b"new").unwrap();
        fs::write(&destination, b"keep").unwrap();
        let error = publish_new_with(no_links, &staged, &destination).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&destination).unwrap(), b"keep");
        #[cfg(unix)]
        {
            let (target, link) = (dir.path().join("elsewhere"), dir.path().join("link"));
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let error = publish_new_with(no_links, &staged, &link).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            assert!(!target.exists());
        }
    }

    #[test]
    fn a_taken_name_is_reported_without_copying() {
        let dir = tempfile::tempdir().unwrap();
        let (staged, destination) = (dir.path().join("staged"), dir.path().join("final"));
        fs::write(&staged, b"new").unwrap();
        fs::write(&destination, b"keep").unwrap();
        assert_eq!(publish_new(&staged, &destination).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&destination).unwrap(), b"keep");
        publish_new(&staged, &dir.path().join("fresh")).unwrap();
        assert_eq!(fs::read(dir.path().join("fresh")).unwrap(), b"new");
    }

    #[test]
    fn a_missing_staging_file_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("final");
        assert!(publish_new(&dir.path().join("absent"), &destination).is_err());
        assert!(publish_new_with(no_links, &dir.path().join("absent"), &destination).is_err());
        assert!(!destination.exists());
    }
}
