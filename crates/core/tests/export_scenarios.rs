use soundshelf_core::{
    catalog::{hash_file, Catalog, Clip, ClipRecipe, Profile},
    export::{render, ExportFormat, ExportManifest, ExportOptions, ExportService},
    media::{analyze, MediaTools},
    search::SearchQuery,
};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, Mutex},
};
use tempfile::{tempdir, TempDir};

fn flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}
struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    source: PathBuf,
    profile: Profile,
    clip: Clip,
    catalog: Mutex<Catalog>,
    tools: MediaTools,
}
fn fixture(rate: u32) -> Fixture {
    let dir = tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let media = root.join("original media");
    fs::create_dir(&media).unwrap();
    let source = media.join("fixture with spaces.wav");
    // Two-channel deterministic signed PCM ramp. Distinct channels detect mapping errors.
    let frames = rate * 2;
    let mut wav = fs::File::create(&source).unwrap();
    wav.write_all(b"RIFF").unwrap();
    wav.write_all(&(36 + frames * 4).to_le_bytes()).unwrap();
    wav.write_all(b"WAVEfmt ").unwrap();
    wav.write_all(&16u32.to_le_bytes()).unwrap();
    wav.write_all(&1u16.to_le_bytes()).unwrap();
    wav.write_all(&2u16.to_le_bytes()).unwrap();
    wav.write_all(&rate.to_le_bytes()).unwrap();
    wav.write_all(&(rate * 4).to_le_bytes()).unwrap();
    wav.write_all(&4u16.to_le_bytes()).unwrap();
    wav.write_all(&16u16.to_le_bytes()).unwrap();
    wav.write_all(b"data").unwrap();
    wav.write_all(&(frames * 4).to_le_bytes()).unwrap();
    for n in 0..frames {
        let sample = ((n % 16000) as i16) - 8000;
        wav.write_all(&sample.to_le_bytes()).unwrap();
        wav.write_all(&(-sample).to_le_bytes()).unwrap();
    }
    drop(wav);
    let tools = MediaTools::discover()
        .expect("Run fixture tests with SOUNDSHELF_FFMPEG and SOUNDSHELF_FFPROBE");
    let profile = analyze(&tools, &source, flag()).unwrap();
    let hash = hash_file(&source).unwrap();
    let mut catalog = Catalog::open(&root.join("catalog.sqlite")).unwrap();
    let scope = catalog.add_source(&media).unwrap();
    let id = catalog
        .register(&scope, "fixture with spaces.wav", &hash)
        .unwrap();
    catalog.publish(&scope, &id, &hash, &profile).unwrap();
    let recipe = ClipRecipe {
        asset_id: id.clone(),
        asset_version_id: hash,
        source_sample_rate_hz: rate,
        start_frame: "5926".into(),
        end_frame: (5926 + rate / 2).to_string(),
        channel_policy: "preserve".into(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = catalog
        .create_clip(&id, "Precise ramp transient", &recipe)
        .unwrap();
    fs::create_dir(root.join("journal")).unwrap();
    fs::create_dir(root.join("exports with spaces")).unwrap();
    Fixture {
        _dir: dir,
        root,
        source,
        profile,
        clip,
        catalog: Mutex::new(catalog),
        tools,
    }
}
fn output(f: &Fixture, extension: &str) -> PathBuf {
    f.root
        .join("exports with spaces")
        .join(format!("selected audio.{extension}"))
}
fn samples(f: &Fixture, path: &std::path::Path) -> Vec<f32> {
    let data = std::process::Command::new(&f.tools.ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "pipe:1"])
        .output()
        .unwrap();
    assert!(data.status.success());
    data.stdout
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
fn clean(f: &Fixture) {
    assert_eq!(fs::read_dir(f.root.join("journal")).unwrap().count(), 0);
    assert!(!fs::read_dir(f.root.join("exports with spaces"))
        .unwrap()
        .any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".soundshelf-export-")));
}
#[test]
#[ignore = "requires FFmpeg fixture tools"]
fn exact_lossless_boundaries_and_manifest_at_44k_and_48k() {
    for rate in [44100, 48000] {
        let f = fixture(rate);
        let before = fs::read(&f.source).unwrap();
        let path = output(&f, "wav");
        let (manifest, p) = render(
            &f.tools,
            &f.source,
            &f.clip,
            &f.profile,
            &path,
            ExportOptions::default(),
            flag(),
            &f.root.join("journal"),
        )
        .unwrap();
        assert_eq!(p.frames, rate as u64 / 2);
        assert_eq!(p.channels, 2);
        let original = samples(&f, &f.source);
        let exported = samples(&f, &path);
        let start = 5926 * 2;
        let end = start + rate as usize;
        assert_eq!(
            exported,
            original[start..end],
            "no preroll or exclusive-end leakage"
        );
        let saved: ExportManifest = serde_json::from_slice(
            &fs::read(format!("{}.soundshelf.json", path.display())).unwrap(),
        )
        .unwrap();
        assert_eq!(saved.frames, manifest.frames);
        assert_eq!(saved.recipe.start_frame, "5926");
        assert_eq!(saved.media_file, "selected audio.wav");
        assert_eq!(hash_file(&path).unwrap(), saved.content_hash);
        assert_eq!(fs::read(&f.source).unwrap(), before);
        clean(&f);
    }
}
#[test]
#[ignore = "requires FFmpeg fixture tools"]
fn resample_flac_fades_gain_and_channel_mapping() {
    let mut f = fixture(48000);
    // Keep stereo and attenuate to detect the gain/fade rather than cancellation.
    f.clip.recipe.gain_db = -6.0;
    let options = ExportOptions {
        format: ExportFormat::Flac,
        sample_rate: Some(44100),
        fade_in_ms: Some(10),
        fade_out_ms: Some(20),
    };
    let path = output(&f, "flac");
    let (manifest, p) = render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        options,
        flag(),
        &f.root.join("journal"),
    )
    .unwrap();
    assert_eq!(p.frames, 22050);
    assert_eq!(p.sample_rate, 44100);
    assert_eq!(p.channels, 2);
    let data = samples(&f, &path);
    assert!(data[0].abs() < 0.005);
    assert!(data[data.len() - 2].abs() < 0.005);
    assert_eq!(manifest.recipe.fade_out_ms, 20);
    let attenuated = p.peak;
    f.clip.recipe.gain_db = 0.0;
    let baseline_path = f.root.join("exports with spaces").join("baseline.flac");
    let (_, baseline) = render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &baseline_path,
        ExportOptions {
            format: ExportFormat::Flac,
            sample_rate: Some(44100),
            fade_in_ms: Some(10),
            fade_out_ms: Some(20),
        },
        flag(),
        &f.root.join("journal"),
    )
    .unwrap();
    assert!((attenuated / baseline.peak - 10f32.powf(-6.0 / 20.0)).abs() < 0.001);
    f.clip.recipe.end_frame = "29925".into();
    f.clip.recipe.channel_policy = "mono".into();
    let (_, mono) = render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &output(&f, "wav"),
        ExportOptions::default(),
        flag(),
        &f.root.join("journal"),
    )
    .unwrap();
    assert_eq!(mono.frames, 23999);
    assert_eq!(mono.channels, 1);
    assert!(mono.peak < 0.0001);
    clean(&f);
}
#[test]
#[ignore = "requires FFmpeg fixture tools"]
fn destination_scope_searchable_variants_and_duplicate_reimport() {
    let f = fixture(48000);
    let service = ExportService::new(f.root.join("journal")).unwrap();
    let grant = service
        .grant(&output(&f, "wav"), ExportFormat::Wav)
        .unwrap();
    assert!(service
        .export(
            &f.catalog,
            &f.tools,
            "forged-grant",
            &f.clip.id,
            1,
            ExportOptions::default()
        )
        .is_err());
    let result = service
        .export(
            &f.catalog,
            &f.tools,
            &grant.id,
            &f.clip.id,
            1,
            ExportOptions::default(),
        )
        .unwrap();
    assert!(result.warning.is_none());
    assert!(result.sound_id.is_some());
    assert!(service
        .export(
            &f.catalog,
            &f.tools,
            &grant.id,
            &f.clip.id,
            1,
            ExportOptions::default()
        )
        .is_err());
    let mut c = f.catalog.lock().unwrap();
    let hits = c
        .search(&SearchQuery {
            text: "Precise ramp transient".into(),
            ..Default::default()
        })
        .unwrap();
    assert!(hits
        .items
        .iter()
        .any(|s| Some(&s.id) == result.sound_id.as_ref()));
    let exported = c.sound(result.sound_id.as_ref().unwrap()).unwrap();
    let source = c.source(&exported.source_id).unwrap();
    let again = c
        .register(&source, &exported.relative_path, &exported.content_hash)
        .unwrap();
    c.publish(
        &source,
        &again,
        &exported.content_hash,
        exported.profile.as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(Some(again), result.sound_id);
    assert_eq!(c.all_sounds().unwrap().len(), 2);
    clean(&f);
}
#[test]
#[ignore = "requires FFmpeg fixture tools"]
fn refuses_overwrites_stale_revisions_replacement_and_invalid_options() {
    let mut f = fixture(48000);
    let path = output(&f, "wav");
    fs::write(&path, b"do not touch").unwrap();
    assert!(render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal")
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"do not touch");
    fs::remove_file(&path).unwrap();
    let sidecar = format!("{}.soundshelf.json", path.display());
    fs::write(&sidecar, b"existing manifest").unwrap();
    assert!(render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal")
    )
    .is_err());
    assert_eq!(fs::read(&sidecar).unwrap(), b"existing manifest");
    fs::remove_file(sidecar).unwrap();
    assert!(render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &f.source,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal")
    )
    .is_err());
    for options in [
        ExportOptions {
            sample_rate: Some(0),
            ..Default::default()
        },
        ExportOptions {
            fade_in_ms: Some(501),
            ..Default::default()
        },
        ExportOptions {
            format: ExportFormat::Flac,
            ..Default::default()
        },
    ] {
        assert!(render(
            &f.tools,
            &f.source,
            &f.clip,
            &f.profile,
            &path,
            options,
            flag(),
            &f.root.join("journal")
        )
        .is_err());
    }
    let service = ExportService::new(f.root.join("journal")).unwrap();
    let grant = service.grant(&path, ExportFormat::Wav).unwrap();
    assert!(service
        .export(
            &f.catalog,
            &f.tools,
            &grant.id,
            &f.clip.id,
            2,
            ExportOptions::default()
        )
        .is_err());
    f.clip.is_stale = true;
    assert!(render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal")
    )
    .is_err());
    f.clip.is_stale = false;
    fs::write(&f.source, b"replaced without scan").unwrap();
    assert!(render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal")
    )
    .unwrap_err()
    .to_string()
    .contains("Source content changed"));
    assert!(!path.exists());
    clean(&f);
}
#[test]
fn grants_are_exact_one_use_cancellable_and_collision_safe() {
    let dir = tempdir().unwrap();
    let service = ExportService::new(dir.path().join("journal")).unwrap();
    assert!(service
        .grant(std::path::Path::new("relative.wav"), ExportFormat::Wav)
        .is_err());
    assert!(service
        .grant(&dir.path().join("wrong.flac"), ExportFormat::Wav)
        .is_err());
    let dest = dir.path().join("output.wav");
    let a = service.grant(&dest, ExportFormat::Wav).unwrap();
    let b = service.grant(&dest, ExportFormat::Wav).unwrap();
    assert!(service.cancel(&a.id).is_err());
    service.cancel(&b.id).unwrap();
    assert!(service.cancel(&b.id).is_err());
    fs::write(&dest, b"original").unwrap();
    assert!(service.grant(&dest, ExportFormat::Wav).is_err());
}
#[test]
fn interrupted_export_cleanup_preserves_unrelated_files() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("journal");
    fs::create_dir(&journal).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let stage = dir.path().join(format!(".soundshelf-export-{id}"));
    fs::create_dir(&stage).unwrap();
    fs::write(stage.join("media"), b"incomplete").unwrap();
    fs::write(stage.join("manifest"), b"pending").unwrap();
    let orphan = dir.path().join("unfinished.wav.soundshelf.json");
    fs::hard_link(stage.join("manifest"), &orphan).unwrap();
    fs::write(
        journal.join(format!("{id}.json")),
        serde_json::to_vec(
            &serde_json::json!({"dir": stage, "destination": dir.path().join("unfinished.wav")}),
        )
        .unwrap(),
    )
    .unwrap();
    let final_file = dir.path().join("finished.wav");
    fs::write(&final_file, b"valid export").unwrap();
    ExportService::new(journal.clone()).unwrap();
    assert!(!stage.exists());
    assert!(!orphan.exists());
    assert_eq!(fs::read(final_file).unwrap(), b"valid export");
    assert_eq!(fs::read_dir(journal).unwrap().count(), 0);
}
#[cfg(unix)]
mod unix_faults {
    use super::*;
    use std::{
        os::unix::fs::{symlink, PermissionsExt},
        time::Duration,
    };
    fn fake(f: &Fixture, script: &str) -> MediaTools {
        let executable = f.root.join("fake ffmpeg");
        fs::write(&executable, script).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        MediaTools {
            ffmpeg: executable,
            ffprobe: f.tools.ffprobe.clone(),
        }
    }
    #[test]
    #[ignore = "requires FFmpeg fixture tools"]
    fn cancel_running_export_kills_process_and_cleans_staging() {
        let f = fixture(48000);
        let service = Arc::new(ExportService::new(f.root.join("journal")).unwrap());
        let tools = fake(&f, "#!/bin/sh\nexec /bin/sleep 60\n");
        let path = output(&f, "wav");
        let grant = service.grant(&path, ExportFormat::Wav).unwrap();
        let catalog = Arc::new(f.catalog);
        let worker_service = service.clone();
        let id = grant.id.clone();
        let clip_id = f.clip.id.clone();
        let worker_catalog = catalog.clone();
        let worker = std::thread::spawn(move || {
            worker_service.export(
                &worker_catalog,
                &tools,
                &id,
                &clip_id,
                1,
                ExportOptions::default(),
            )
        });
        let started = std::time::Instant::now();
        while fs::read_dir(f.root.join("journal")).unwrap().count() == 0 {
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(service
            .grant(&f.root.join("another.wav"), ExportFormat::Wav)
            .is_err());
        service.cancel(&grant.id).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(f.root.join("journal")).unwrap().count(), 0);
    }
    #[test]
    #[ignore = "requires FFmpeg fixture tools"]
    fn disk_full_and_corrupt_output_leave_no_final_media() {
        let f = fixture(48000);
        let path = output(&f, "wav");
        // Inject the encoder failure used for disk exhaustion, without filling the user's drive.
        let tools = fake(
            &f,
            "#!/bin/sh\nprintf 'No space left on device\\n' >&2\nexit 1\n",
        );
        assert!(render(
            &tools,
            &f.source,
            &f.clip,
            &f.profile,
            &path,
            ExportOptions::default(),
            flag(),
            &f.root.join("journal")
        )
        .is_err());
        assert!(!path.exists());
        clean(&f);
        let tools = fake(
            &f,
            "#!/bin/sh\nfor arg do last=$arg; done\nprintf corrupt > \"$last\"\n",
        );
        assert!(render(
            &tools,
            &f.source,
            &f.clip,
            &f.profile,
            &path,
            ExportOptions::default(),
            flag(),
            &f.root.join("journal")
        )
        .is_err());
        assert!(!path.exists());
        clean(&f);
    }
    #[test]
    #[ignore = "requires FFmpeg fixture tools"]
    fn symlink_destination_and_commit_collision_never_overwrite() {
        let f = fixture(48000);
        let path = output(&f, "wav");
        let before = fs::read(&f.source).unwrap();
        symlink(&f.source, &path).unwrap();
        assert!(render(
            &f.tools,
            &f.source,
            &f.clip,
            &f.profile,
            &path,
            ExportOptions::default(),
            flag(),
            &f.root.join("journal")
        )
        .is_err());
        assert_eq!(fs::read(&f.source).unwrap(), before);
        fs::remove_file(&path).unwrap();
        // Collision arrives after the preflight check while FFmpeg renders.
        let collision = path.clone();
        let journal = f.root.join("journal");
        let thread = std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while fs::read_dir(&journal).unwrap().count() == 0 {
                assert!(start.elapsed() < Duration::from_secs(5));
                std::thread::sleep(Duration::from_millis(1));
            }
            fs::write(collision, b"racing writer").unwrap();
        });
        assert!(render(
            &f.tools,
            &f.source,
            &f.clip,
            &f.profile,
            &path,
            ExportOptions::default(),
            flag(),
            &f.root.join("journal")
        )
        .is_err());
        thread.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"racing writer");
        assert!(!PathBuf::from(format!("{}.soundshelf.json", path.display())).exists());
        clean(&f);
    }
}

#[test]
fn recovery_ignores_corrupt_and_unowned_journal_entries() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("journal");
    fs::create_dir(&journal).unwrap();
    let unrelated = dir.path().join("user folder");
    fs::create_dir(&unrelated).unwrap();
    fs::write(unrelated.join("media"), b"original").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    fs::write(
        journal.join(format!("{id}.json")),
        serde_json::to_vec(
            &serde_json::json!({"dir": unrelated, "destination": dir.path().join("output.wav")}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(
        journal.join(format!("{}.json", uuid::Uuid::new_v4())),
        b"truncated json",
    )
    .unwrap();
    ExportService::new(journal).unwrap();
    assert_eq!(fs::read(unrelated.join("media")).unwrap(), b"original");
}

#[test]
#[ignore = "requires FFmpeg fixture tools"]
fn pre_cancel_and_single_frame_export() {
    let mut f = fixture(48000);
    let path = output(&f, "wav");
    let cancelled = Arc::new(AtomicBool::new(true));
    assert!(render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        cancelled,
        &f.root.join("journal")
    )
    .is_err());
    assert!(!path.exists());
    clean(&f);
    f.clip.recipe.end_frame = "5927".into();
    let (_, p) = render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal"),
    )
    .unwrap();
    assert_eq!(p.frames, 1);
    assert_eq!(samples(&f, &path), samples(&f, &f.source)[11852..11854]);
    clean(&f);
}

#[test]
#[ignore = "requires FFmpeg fixture tools"]
fn watched_source_export_reuses_scope_and_preserves_annotations_on_reindex() {
    let f = fixture(48000);
    let path = f.source.parent().unwrap().join("variant.wav");
    let (manifest, p) = render(
        &f.tools,
        &f.source,
        &f.clip,
        &f.profile,
        &path,
        ExportOptions::default(),
        flag(),
        &f.root.join("journal"),
    )
    .unwrap();
    let mut c = f.catalog.lock().unwrap();
    let id = c.index_export(&path, &manifest, &p).unwrap();
    c.annotate(&id, &["edited".into()], "User note", true)
        .unwrap();
    assert_eq!(c.index_export(&path, &manifest, &p).unwrap(), id);
    assert_eq!(c.sources().unwrap().len(), 1);
    assert_eq!(c.all_sounds().unwrap().len(), 2);
    let updated = c.sound(&id).unwrap();
    assert_eq!(updated.comment, "User note");
    assert!(updated.favorite);
    assert_eq!(updated.user_tags, vec!["edited"]);
}

#[test]
fn interrupted_export_keeps_journal_until_destination_returns() {
    let dir = tempdir().unwrap();
    let journal = dir.path().join("journal");
    fs::create_dir(&journal).unwrap();
    let drive = dir.path().join("drive");
    fs::create_dir(&drive).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let stage = drive.join(format!(".soundshelf-export-{id}"));
    fs::create_dir(&stage).unwrap();
    fs::write(stage.join("media"), b"partial").unwrap();
    fs::write(
        journal.join(format!("{id}.json")),
        serde_json::to_vec(
            &serde_json::json!({"dir": stage, "destination": drive.join("output.wav")}),
        )
        .unwrap(),
    )
    .unwrap();
    let unplugged = dir.path().join("unplugged");
    fs::rename(&drive, &unplugged).unwrap();
    ExportService::new(journal.clone()).unwrap();
    assert_eq!(fs::read_dir(&journal).unwrap().count(), 1);
    fs::rename(&unplugged, &drive).unwrap();
    ExportService::new(journal.clone()).unwrap();
    assert!(!stage.exists());
    assert_eq!(fs::read_dir(journal).unwrap().count(), 0);
}
