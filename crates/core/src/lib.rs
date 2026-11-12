pub mod catalog;
pub mod search;
pub mod media;
pub mod library;
pub mod jobs;
pub mod playback;
pub mod waveform;
pub mod export;
pub mod publish;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn invalid(message: &str) -> Error { Error::Invalid(message.into()) }

/// Create the folder that holds the catalog, caches, MCP pairings and provider
/// settings, and keep it private to the current user. Files inside are created
/// with the process umask, so on Unix the folder itself is what stops other
/// local accounts reading file paths, notes and prompts. An existing folder
/// with wider access is narrowed. Windows profiles are already per-user.
pub fn private_directory(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        if mode & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7700))?;
        }
    }
    Ok(())
}

pub mod sources;

pub mod portable;
pub mod mixed_catalog;
pub mod ai_provider;
pub mod ai_provider_wire;
pub mod image_ai;
pub mod image_eval;

pub mod agent;
pub mod agent_images;

pub mod source_catalog;

pub mod handoff;

pub mod maintenance;

// Data contracts only; image services are introduced by later tickets.
pub mod image_contracts;
pub mod image_catalog;

#[cfg(test)]
mod image_migration_tests;

pub mod image_analysis;
mod image_decode;

pub mod image_jobs;

pub mod image_import_preview;

pub mod image_import;

pub mod image_preview;

pub mod image_gallery;
pub mod image_annotations;
pub mod image_search;
pub mod image_duplicates;
/// SS-055 generation metadata reader and prompt provenance.
pub mod image_generation;
pub mod image_provenance;
mod image_preview_render;

pub mod image_sources;
pub mod image_relink;
pub mod image_watch;

#[cfg(all(test, unix))]
mod private_directory_tests {
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn data_folder_is_owner_only_and_an_open_one_is_narrowed() {
        let base = std::env::temp_dir().join(format!("creativeshelf-private-{}", uuid::Uuid::new_v4()));
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        // A folder left world-readable by an earlier version is narrowed in place.
        let existing = base.join("existing");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(existing.join("library.sqlite"), b"kept").unwrap();
        super::private_directory(&existing).unwrap();
        assert_eq!(mode(&existing), 0o700);
        assert_eq!(std::fs::read(existing.join("library.sqlite")).unwrap(), b"kept");
        // A new folder, including missing parents, ends up owner-only.
        let fresh = base.join("a/b/fresh");
        super::private_directory(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        // Already-private folders are left exactly as they are.
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o500)).unwrap();
        super::private_directory(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o500);
        std::fs::set_permissions(&fresh, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }
}
