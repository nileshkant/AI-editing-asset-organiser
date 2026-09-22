use rusqlite::Connection;
use soundshelf_core::catalog::{
    parse_frame, validate_clip_recipe, Catalog, ClipRecipe, Profile, SCHEMA_VERSION,
};
use std::fs::{create_dir_all, File};
use std::io::Write;
use tempfile::tempdir;

fn test_profile(sample_rate: u32, duration_secs: f64) -> Profile {
    let frames = (duration_secs * sample_rate as f64).round() as u64;
    Profile {
        duration: duration_secs,
        sample_rate,
        channels: 2,
        frames,
        peak: 0.85,
        rms: 0.25,
        channel_peaks: vec![0.85, 0.82],
        channel_rms: vec![0.25, 0.24],
        channel_layout: "stereo".to_string(),
        description: "Calibrated test clip tone".to_string(),
        tags: vec!["test".to_string(), "synth".to_string()],
        waveform: vec![[-0.85, 0.85], [-0.5, 0.5]],
    }
}

fn setup_catalog_with_sound(sample_rate: u32, duration_secs: f64) -> (tempfile::TempDir, Catalog, String, String) {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&db_path).unwrap();

    let media_dir = dir.path().join("media");
    create_dir_all(&media_dir).unwrap();
    let file_path = media_dir.join("sample.wav");
    File::create(&file_path).unwrap().write_all(b"RIFF dummy audio data").unwrap();

    let source = catalog.add_source(&media_dir).unwrap();
    let hash = "hash-version-alpha-123";
    let sound_id = catalog.register(&source, "sample.wav", hash).unwrap();
    let profile = test_profile(sample_rate, duration_secs);
    catalog.publish(&source, &sound_id, hash, &profile).unwrap();

    (dir, catalog, sound_id, hash.to_string())
}

#[test]
fn v3_catalogs_gain_clips_table_transactionally() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("v3.sqlite");
    {
        // Construct a v3 database manually
        let mut db = Connection::open(&db_path).unwrap();
        let tx = db.transaction().unwrap();
        tx.execute_batch(
            "CREATE TABLE sources (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE,
  generation INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1 CHECK(available IN(0,1))
);
CREATE TABLE sounds (
  id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES sources(id),
  relative_path TEXT NOT NULL, title TEXT NOT NULL, content_hash TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN('pending','ready','failed','missing')),
  UNIQUE(source_id,relative_path)
);
CREATE INDEX sounds_hash ON sounds(content_hash);
CREATE INDEX sounds_state ON sounds(source_id,status);
CREATE TABLE analyses (
  content_hash TEXT NOT NULL, analyzer TEXT NOT NULL, profile TEXT NOT NULL,
  PRIMARY KEY(content_hash,analyzer)
);
CREATE TABLE annotations (
  sound_id TEXT PRIMARY KEY REFERENCES sounds(id), tags TEXT NOT NULL DEFAULT '[]',
  comment TEXT NOT NULL DEFAULT '', favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN(0,1))
);
CREATE TABLE jobs (
  id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES sources(id),
  kind TEXT NOT NULL, state TEXT NOT NULL, status TEXT NOT NULL,
  lease_owner TEXT, lease_until INTEGER, completed INTEGER NOT NULL DEFAULT 0,
  total INTEGER NOT NULL DEFAULT 0, reused INTEGER NOT NULL DEFAULT 0,
  failed INTEGER NOT NULL DEFAULT 0, current TEXT NOT NULL DEFAULT '',
  errors TEXT NOT NULL DEFAULT '[]', created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  UNIQUE(source_id, kind)
);
CREATE TABLE saved_searches (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, query TEXT NOT NULL, created_at INTEGER NOT NULL
);",
        )
        .unwrap();
        tx.pragma_update(None, "user_version", 3).unwrap();
        tx.commit().unwrap();
    }

    // Open with Catalog — should migrate to v4
    let catalog = Catalog::open(&db_path).unwrap();
    let version: u32 = catalog
        .db_connection()
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    assert_eq!(version, 4);

    // Verify clips table is queryable
    let count: i64 = catalog
        .db_connection()
        .query_row("SELECT count(*) FROM clips", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn sample_rate_and_decimal_string_counters_44k_and_48k() {
    // 48 kHz test: 12.5s start is 600,000 frames. 15s duration is 720,000 frames. End is 1,320,000 exclusive.
    let (_dir48, mut catalog48, sound_id48, hash48) = setup_catalog_with_sound(48000, 30.0);
    let recipe48 = ClipRecipe {
        asset_id: sound_id48.clone(),
        asset_version_id: hash48,
        source_sample_rate_hz: 48000,
        start_frame: "600000".to_string(),
        end_frame: "1320000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 10,
        fade_out_ms: 10,
    };
    let clip48 = catalog48.create_clip(&sound_id48, "48k 15s Stinger", &recipe48).unwrap();
    assert_eq!(clip48.recipe.start_frame, "600000");
    assert_eq!(clip48.recipe.end_frame, "1320000");
    assert_eq!(clip48.revision, 1);
    assert!(!clip48.is_stale);

    // 44.1 kHz test: 10.0s start is 441,000 frames. 15s duration is 661,500 frames. End is 1,102,500.
    let (_dir44, mut catalog44, sound_id44, hash44) = setup_catalog_with_sound(44100, 30.0);
    let recipe44 = ClipRecipe {
        asset_id: sound_id44.clone(),
        asset_version_id: hash44,
        source_sample_rate_hz: 44100,
        start_frame: "441000".to_string(),
        end_frame: "1102500".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: -1.5,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip44 = catalog44.create_clip(&sound_id44, "44.1k 15s Clip", &recipe44).unwrap();
    assert_eq!(clip44.recipe.start_frame, "441000");
    assert_eq!(clip44.recipe.end_frame, "1102500");
    assert!(!clip44.is_stale);
}

#[test]
fn validation_rejects_zero_negative_out_of_range_nan() {
    let (_dir, mut catalog, sound_id, hash) = setup_catalog_with_sound(48000, 10.0);
    // Total frames = 480,000

    // 1. start == end (zero duration)
    let zero_dur = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "1000".to_string(),
        end_frame: "1000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "Zero", &zero_dur).is_err());

    // 2. start > end (negative duration)
    let inverted = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "5000".to_string(),
        end_frame: "1000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "Inverted", &inverted).is_err());

    // 3. end > source total frames (480,000)
    let out_of_range = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "1000".to_string(),
        end_frame: "500000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "Exceeds", &out_of_range).is_err());

    // 4. Non-integer / NaN string
    let nan_frame = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "NaN".to_string(),
        end_frame: "1000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "NaN", &nan_frame).is_err());

    // 5. Negative string
    let neg_frame = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "-100".to_string(),
        end_frame: "1000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "Neg", &neg_frame).is_err());

    // 6. Non-finite gain_db
    let inf_gain = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "0".to_string(),
        end_frame: "1000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: f32::NAN,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "Inf Gain", &inf_gain).is_err());

    // 7. Mismatched asset_version_id
    let stale_version = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: "mismatched-old-version".to_string(),
        source_sample_rate_hz: 48000,
        start_frame: "0".to_string(),
        end_frame: "1000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    assert!(catalog.create_clip(&sound_id, "Old Version", &stale_version).is_err());
}

#[test]
fn fractional_seconds_and_exact_rounding() {
    let sample_rate = 48000u32;
    // 0.123456 seconds -> 5925.888 frames -> rounds to 5926
    let seconds = 0.123456f64;
    let rounded_frames = (seconds * sample_rate as f64).round() as u64;
    assert_eq!(rounded_frames, 5926);

    let parsed = parse_frame(&rounded_frames.to_string()).unwrap();
    assert_eq!(parsed, 5926);

    let recipe = ClipRecipe {
        asset_id: "test".to_string(),
        asset_version_id: "ver".to_string(),
        source_sample_rate_hz: sample_rate,
        start_frame: "0".to_string(),
        end_frame: rounded_frames.to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let (s, e) = validate_clip_recipe(&recipe, Some(10000)).unwrap();
    assert_eq!(s, 0);
    assert_eq!(e, 5926);
}

#[test]
fn source_replacement_flags_stale_recipes() {
    let (_dir, mut catalog, sound_id, initial_hash) = setup_catalog_with_sound(48000, 20.0);

    let recipe = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: initial_hash,
        source_sample_rate_hz: 48000,
        start_frame: "0".to_string(),
        end_frame: "480000".to_string(), // 10s
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = catalog.create_clip(&sound_id, "Variant 1", &recipe).unwrap();
    assert!(!clip.is_stale);

    // Simulate replacing the file with new content: update sound content_hash
    let new_hash = "hash-version-beta-456";
    catalog
        .db_connection_mut()
        .execute(
            "UPDATE sounds SET content_hash = ?2 WHERE id = ?1",
            rusqlite::params![sound_id, new_hash],
        )
        .unwrap();

    // Query the clip — it should now be flagged stale!
    let fetched = catalog.get_clip(&clip.id).unwrap();
    assert!(fetched.is_stale);
    assert!(fetched
        .stale_reason
        .as_ref()
        .unwrap()
        .contains("Source content version changed"));

    // Also verify listing clips flags it
    let sound_clips = catalog.list_clips_for_sound(&sound_id).unwrap();
    assert_eq!(sound_clips.len(), 1);
    assert!(sound_clips[0].is_stale);
}

#[test]
fn source_shortened_flags_stale_recipes() {
    let (_dir, mut catalog, sound_id, hash) = setup_catalog_with_sound(48000, 20.0);
    // Source is 20s (960,000 frames)

    let recipe = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "480000".to_string(), // 10s
        end_frame: "864000".to_string(),   // 18s
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = catalog.create_clip(&sound_id, "Outro 18s", &recipe).unwrap();
    assert!(!clip.is_stale);

    // Now update profile to a shortened 12s file (576,000 frames)
    let shortened_profile = test_profile(48000, 12.0);
    catalog
        .db_connection_mut()
        .execute(
            "UPDATE analyses SET profile = ?2 WHERE content_hash = ?1",
            rusqlite::params![hash, serde_json::to_string(&shortened_profile).unwrap()],
        )
        .unwrap();

    let fetched = catalog.get_clip(&clip.id).unwrap();
    assert!(fetched.is_stale);
    assert!(fetched
        .stale_reason
        .as_ref()
        .unwrap()
        .contains("Recipe boundaries exceed shortened source"));
}

#[test]
fn rebind_clip_restores_freshness_and_creates_revision() {
    let (_dir, mut catalog, sound_id, initial_hash) = setup_catalog_with_sound(48000, 20.0);

    let recipe = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: initial_hash,
        source_sample_rate_hz: 48000,
        start_frame: "0".to_string(),
        end_frame: "480000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = catalog.create_clip(&sound_id, "Theme Stinger", &recipe).unwrap();
    assert_eq!(clip.revision, 1);

    // Modify source content
    let new_hash = "hash-updated-content-789";
    catalog
        .db_connection_mut()
        .execute(
            "UPDATE sounds SET content_hash = ?2 WHERE id = ?1",
            rusqlite::params![sound_id, new_hash],
        )
        .unwrap();
    // Publish profile for new hash
    let new_profile = test_profile(48000, 20.0);
    catalog
        .db_connection_mut()
        .execute(
            "INSERT INTO analyses (content_hash, analyzer, profile) VALUES (?1, 'soundshelf-pcm-v1', ?2)",
            rusqlite::params![new_hash, serde_json::to_string(&new_profile).unwrap()],
        )
        .unwrap();

    let stale = catalog.get_clip(&clip.id).unwrap();
    assert!(stale.is_stale);

    // Perform explicit rebind
    let rebound = catalog.rebind_clip(&clip.id).unwrap();
    assert!(!rebound.is_stale);
    assert_eq!(rebound.revision, 2);
    assert_eq!(rebound.recipe.asset_version_id, new_hash);

    // Verify revisions table recorded both revision 1 and 2
    let rev_count: i64 = catalog
        .db_connection()
        .query_row(
            "SELECT count(*) FROM clip_revisions WHERE clip_id = ?1",
            [&clip.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rev_count, 2);
}

#[test]
fn revision_concurrency_conflict_detected() {
    let (_dir, mut catalog, sound_id, hash) = setup_catalog_with_sound(48000, 20.0);

    let recipe = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash.clone(),
        source_sample_rate_hz: 48000,
        start_frame: "0".to_string(),
        end_frame: "100000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = catalog.create_clip(&sound_id, "Initial Name", &recipe).unwrap();

    // Client A updates clip successfully (bumps revision to 2)
    let updated_recipe = ClipRecipe {
        gain_db: 2.0,
        ..recipe.clone()
    };
    catalog
        .update_clip(&clip.id, "Client A Name", &updated_recipe, 1)
        .unwrap();

    // Client B attempts to update using stale expected_revision = 1 -> should fail with conflict!
    let conflict_err = catalog
        .update_clip(&clip.id, "Client B Name", &recipe, 1)
        .unwrap_err();
    assert!(conflict_err.to_string().contains("revision conflict"));
}

#[test]
fn clip_deletion_removes_clip_and_revisions() {
    let (_dir, mut catalog, sound_id, hash) = setup_catalog_with_sound(48000, 10.0);

    let recipe = ClipRecipe {
        asset_id: sound_id.clone(),
        asset_version_id: hash,
        source_sample_rate_hz: 48000,
        start_frame: "0".to_string(),
        end_frame: "48000".to_string(),
        channel_policy: "preserve".to_string(),
        gain_db: 0.0,
        fade_in_ms: 0,
        fade_out_ms: 0,
    };
    let clip = catalog.create_clip(&sound_id, "To Delete", &recipe).unwrap();

    catalog.delete_clip(&clip.id).unwrap();

    assert!(catalog.get_clip(&clip.id).is_err());
    let remaining = catalog.list_clips_for_sound(&sound_id).unwrap();
    assert_eq!(remaining.len(), 0);

    // Re-deleting returns error
    assert!(catalog.delete_clip(&clip.id).is_err());
}
