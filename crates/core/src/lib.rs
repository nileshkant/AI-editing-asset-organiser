pub mod catalog;
pub mod search;
pub mod media;
pub mod library;
pub mod jobs;
pub mod playback;
pub mod waveform;
pub mod export;

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
mod image_preview_render;

pub mod image_sources;
pub mod image_relink;
pub mod image_watch;
