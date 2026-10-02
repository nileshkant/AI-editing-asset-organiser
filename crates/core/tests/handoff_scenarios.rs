use serde_json::{json, Value};
use soundshelf_core::handoff::remotion_handoff;
use std::fs;
fn pair(root: &std::path::Path) -> std::path::PathBuf {
    let audio = b"deterministic handoff bytes";
    fs::write(root.join("rain #1.wav"), audio).unwrap();
    let manifest = json!({"schema_version":1,"media_file":"rain #1.wav","content_hash":blake3::hash(audio).to_hex().to_string(),"sample_rate":48000,"channels":2,"frames":"720000","clip_id":uuid::Uuid::new_v4().to_string(),"clip_revision":2,"name":"rain","recipe":{"asset_id":"fixture","asset_version_id":"fixture","source_sample_rate_hz":48000,"start_frame":"0","end_frame":"720000","channel_policy":"preserve","gain_db":0,"fade_in_ms":0,"fade_out_ms":0},"options":{"format":"wav","sample_rate":null,"fade_in_ms":null,"fade_out_ms":null}});
    let sidecar = root.join("rain.soundshelf.json");
    fs::write(&sidecar, serde_json::to_vec(&manifest).unwrap()).unwrap();
    sidecar
}
#[test]
fn relocation_retains_portable_paths_and_sample_duration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("public");
    fs::create_dir(&root).unwrap();
    let sidecar = pair(&root);
    let props = remotion_handoff(&sidecar, &root).unwrap();
    assert_eq!(props.media, "rain #1.wav");
    assert_eq!(props.duration_seconds, 15.0);
    let moved = temp.path().join("moved public");
    fs::rename(&root, &moved).unwrap();
    assert_eq!(
        props,
        remotion_handoff(&moved.join(sidecar.file_name().unwrap()), &moved).unwrap()
    );
}
#[test]
fn refuses_changed_audio_and_hostile_manifest_fields() {
    let temp = tempfile::tempdir().unwrap();
    let sidecar = pair(temp.path());
    let original: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    for (key, value) in [
        ("media_file", json!("../outside.wav")),
        ("media_file", json!("https://host/audio.wav")),
        ("media_file", json!("C:\\audio.wav")),
        ("frames", json!("9007199254740992")),
        ("frames", json!("-1")),
        ("schema_version", json!(2)),
    ] {
        let mut manifest = original.clone();
        manifest[key] = value;
        fs::write(&sidecar, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(remotion_handoff(&sidecar, temp.path()).is_err());
    }
    fs::write(&sidecar, serde_json::to_vec(&original).unwrap()).unwrap();
    fs::write(temp.path().join("rain #1.wav"), b"changed").unwrap();
    assert!(remotion_handoff(&sidecar, temp.path())
        .unwrap_err()
        .to_string()
        .contains("hash does not match"));
}
#[test]
fn refuses_outside_public_and_oversized_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let sidecar = pair(temp.path());
    assert!(remotion_handoff(&sidecar, other.path()).is_err());
    fs::write(&sidecar, vec![b' '; 65537]).unwrap();
    assert!(remotion_handoff(&sidecar, temp.path()).is_err());
}
#[cfg(unix)]
#[test]
fn rejects_external_media_symlink_and_fifo_without_waiting() {
    let temp = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let sidecar = pair(temp.path());
    pair(other.path());
    let audio = temp.path().join("rain #1.wav");
    fs::remove_file(&audio).unwrap();
    std::os::unix::fs::symlink(other.path().join("rain #1.wav"), &audio).unwrap();
    assert!(remotion_handoff(&sidecar, temp.path()).is_err());
    fs::remove_file(&audio).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(audio.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(remotion_handoff(&sidecar, temp.path()).is_err());
}
#[test]
fn cli_never_overwrites_existing_props() {
    let temp = tempfile::tempdir().unwrap();
    let sidecar = pair(temp.path());
    let output = temp.path().join("props.json");
    fs::write(&output, b"keep").unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_creativeshelf-handoff"))
        .arg(sidecar)
        .arg(temp.path())
        .arg(&output)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(output).unwrap(), b"keep");
}
