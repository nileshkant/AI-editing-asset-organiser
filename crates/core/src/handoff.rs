//! Portable props for already rendered audio; never imports an editor or copies media.
use crate::{export::ExportManifest, invalid, Result};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::Path,
};

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioHandoff {
    pub schema: &'static str,
    pub media: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_frames: String,
    pub duration_seconds: f64,
    pub clip_id: String,
    pub clip_revision: u32,
    pub content_hash: String,
}
fn regular(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("Handoff requires regular files"));
    }
    Ok(file)
}
pub fn remotion_handoff(sidecar: &Path, public_root: &Path) -> Result<AudioHandoff> {
    let root = public_root.canonicalize()?;
    let sidecar = sidecar.canonicalize()?;
    if !root.is_dir() || !sidecar.starts_with(&root) {
        return Err(invalid("Place the exported pair inside Remotion public/"));
    }
    let mut file = regular(&sidecar)?;
    if file.metadata()?.len() > 65536 {
        return Err(invalid("Export manifest exceeds 64 KiB"));
    }
    let mut bytes = Vec::new();
    file.by_ref().take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(invalid("Export manifest exceeds 64 KiB"));
    }
    let manifest: ExportManifest = serde_json::from_slice(&bytes)?;
    let name = &manifest.media_file;
    let frames = manifest
        .frames
        .parse::<u64>()
        .map_err(|_| invalid("Invalid frame count"))?;
    if manifest.schema_version != 1
        || name.is_empty()
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
        || !matches!(
            Path::new(name).extension().and_then(|s| s.to_str()),
            Some("wav" | "flac")
        )
        || frames == 0
        || frames > 9_007_199_254_740_991
        || manifest.frames != frames.to_string()
        || !(8000..=192000).contains(&manifest.sample_rate)
        || !(1..=64).contains(&manifest.channels)
        || manifest.clip_revision == 0
        || uuid::Uuid::parse_str(&manifest.clip_id).is_err()
        || manifest.content_hash.len() != 64
        || !manifest.content_hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("Unsupported or invalid export manifest"));
    }
    let media = sidecar
        .parent()
        .ok_or_else(|| invalid("Missing parent"))?
        .join(name)
        .canonicalize()?;
    let relative = media
        .strip_prefix(&root)
        .map_err(|_| invalid("Audio escapes Remotion public/"))?;
    let mut audio = regular(&media)?;
    let before = audio.metadata()?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = audio.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let after = audio.metadata()?;
    if before.len() != after.len()
        || before.modified()? != after.modified()?
        || hash.finalize().to_hex().as_str() != manifest.content_hash
    {
        return Err(invalid("Exported audio changed or its hash does not match"));
    }
    let media = relative
        .components()
        .map(|c| {
            c.as_os_str()
                .to_str()
                .ok_or_else(|| invalid("Non-Unicode media path"))
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    Ok(AudioHandoff {
        schema: "creativeshelf-remotion-audio/v1",
        media,
        sample_rate: manifest.sample_rate,
        channels: manifest.channels,
        sample_frames: manifest.frames,
        duration_seconds: frames as f64 / manifest.sample_rate as f64,
        clip_id: manifest.clip_id,
        clip_revision: manifest.clip_revision,
        content_hash: manifest.content_hash,
    })
}
