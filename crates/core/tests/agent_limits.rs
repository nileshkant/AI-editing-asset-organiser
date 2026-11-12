//! A long MCP session must not exhaust the agent's idempotency memory.
use serde_json::{json, Value};
use soundshelf_core::{
    agent::{Access, AgentLibrary},
    catalog::{hash_file, Catalog, Profile},
    export::ExportService,
};
use std::{
    fs,
    sync::{Arc, Mutex},
};

struct Fixture {
    library: Arc<AgentLibrary>,
    access: Access,
    sound: String,
    version: String,
    _dir: tempfile::TempDir,
}
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("media")).unwrap();
    fs::write(root.join("media/a.wav"), b"fixture").unwrap();
    let mut catalog = Catalog::open(&root.join("catalog.sqlite")).unwrap();
    let source = catalog.add_source(&root.join("media")).unwrap();
    let version = hash_file(&root.join("media/a.wav")).unwrap();
    let sound = catalog.register(&source, "a.wav", &version).unwrap();
    let profile = Profile {
        duration: 1.0,
        sample_rate: 48000,
        channels: 1,
        frames: 48000,
        peak: 0.5,
        rms: 0.2,
        channel_peaks: vec![0.5],
        channel_rms: vec![0.2],
        channel_layout: "mono".into(),
        description: "Measured fixture".into(),
        tags: vec![],
        waveform: vec![[-0.5, 0.5]],
    };
    catalog.publish(&source, &sound, &version, &profile).unwrap();
    let exports = Arc::new(ExportService::new(root.join("journal")).unwrap());
    Fixture {
        library: Arc::new(AgentLibrary::new(Arc::new(Mutex::new(catalog)), exports, None)),
        access: Access { source_ids: vec![source.id], read: true, edit: true, ..Default::default() },
        sound,
        version,
        _dir: dir,
    }
}
impl Fixture {
    fn create(&self, owner: &str, key: &str) -> Result<Value, &'static str> {
        self.library
            .execute(
                owner,
                &self.access,
                "clips.create",
                json!({"sound_id":self.sound,"name":format!("clip {key}"),"idempotency_key":key,
                    "recipe":{"asset_id":self.sound,"asset_version_id":self.version,
                        "source_sample_rate_hz":48000,"start_frame":"0","end_frame":"10"}}),
            )
            .map_err(|e| e.code)
    }
}

#[test]
fn idempotent_creates_keep_working_past_a_thousand_calls() {
    let f = fixture();
    let other = f.create("other", "kept").unwrap();
    for i in 0..1100 {
        assert_eq!(f.create("busy", &format!("key-{i}")).map(|_| ()), Ok(()), "call {i}");
    }
    // A recent key still replays its original result instead of creating again.
    let again = f.create("busy", "key-1099").unwrap();
    let clips = f.library.catalog.lock().unwrap().list_clips_for_sound(&f.sound).unwrap();
    assert_eq!(clips.len(), 1101);
    assert_eq!(clips.iter().filter(|c| c.id == again["id"].as_str().unwrap()).count(), 1);
    // One busy client never pushes out another client's keys.
    assert_eq!(f.create("other", "kept").unwrap()["id"], other["id"]);
    assert_eq!(f.library.catalog.lock().unwrap().list_clips_for_sound(&f.sound).unwrap().len(), 1101);
}
