use soundshelf_core::{
    catalog::{hash_file, Catalog, Profile, Sound, Source},
    search::{search, SearchQuery},
};
use std::{fs, path::Path, time::Instant};
use tempfile::tempdir;

fn test_profile(duration: f64, channels: u16, layout: &str, tags: Vec<String>) -> Profile {
    Profile {
        duration,
        sample_rate: 48000,
        channels,
        frames: (duration * 48000.0) as u64,
        peak: 0.8,
        rms: 0.2,
        channel_peaks: vec![0.8; channels as usize],
        channel_rms: vec![0.2; channels as usize],
        channel_layout: layout.into(),
        description: "Test audio profile".into(),
        tags,
        waveform: vec![[-0.5, 0.5]],
    }
}

fn make_sound(id: &str, title: &str, duration: f64, user_tags: Vec<String>, comment: &str) -> Sound {
    Sound {
        id: id.into(),
        source_id: "src_1".into(),
        relative_path: format!("{title}.wav"),
        title: title.into(),
        content_hash: format!("hash_{id}"),
        status: "ready".into(),
        profile: Some(test_profile(duration, 2, "stereo", vec![])),
        user_tags,
        comment: comment.into(),
        favorite: false,
    }
}

#[test]
fn exact_matches_rank_first() {
    let s1 = make_sound("1", "Deep Space Heavy Whoosh with Reverb", 1.5, vec![], "");
    let s2 = make_sound("2", "Whoosh", 1.5, vec![], "");
    let s3 = make_sound("3", "Whoosh Impact", 1.5, vec![], "");

    let r = search(vec![s1, s2, s3], &SearchQuery { text: "whoosh".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r.total, 3);
    assert_eq!(r.items[0].title, "Whoosh", "Exact title match must rank first");

    // Exact tag match ranking
    let s4 = make_sound("4", "FX 101", 1.5, vec!["laser blast".into()], "");
    let s5 = make_sound("5", "Laser Blast", 1.5, vec![], "");
    let s6 = make_sound("6", "Sci-Fi Laser Blast Explosive Impact", 1.5, vec![], "");

    let r2 = search(vec![s4, s5, s6], &SearchQuery { text: "laser blast".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r2.items[0].title, "Laser Blast", "Exact title match ranks ahead of tag match and partial match");
}

#[test]
fn quoted_phrases_exact_matching() {
    let s1 = make_sound("1", "Heavy Laser Blast", 1.0, vec![], "SciFi shot");
    let s2 = make_sound("2", "Laser Rifle with Massive Blast", 1.0, vec![], "Rifle sound");
    let s3 = make_sound("3", "Sound with blast and laser", 1.0, vec![], "Separated words");

    // Quoted phrase: only contiguous "laser blast"
    let q_quoted = SearchQuery { text: r#""laser blast""#.into(), ..Default::default() };
    let r_quoted = search(vec![s1.clone(), s2.clone(), s3.clone()], &q_quoted, &["src_1".into()]).unwrap();
    assert_eq!(r_quoted.total, 1);
    assert_eq!(r_quoted.items[0].title, "Heavy Laser Blast");

    // Unquoted terms: all terms required, anywhere
    let q_unquoted = SearchQuery { text: "laser blast".into(), ..Default::default() };
    let r_unquoted = search(vec![s1, s2, s3], &q_unquoted, &["src_1".into()]).unwrap();
    assert_eq!(r_unquoted.total, 3);
}

#[test]
fn negative_filter_syntax() {
    let s1 = make_sound("1", "Gentle Rain", 2.0, vec![], "pure nature sound");
    let s2 = make_sound("2", "Rain with Speech", 2.0, vec![], "contains vocal narration");
    let s3 = make_sound("3", "Heavy Rain with Thunder", 2.0, vec![], "thunderclap boom");

    // Natural language negation
    let r_nat = search(
        vec![s1.clone(), s2.clone(), s3.clone()],
        &SearchQuery { text: "rain without vocal".into(), ..Default::default() },
        &["src_1".into()],
    ).unwrap();
    assert_eq!(r_nat.total, 2);
    assert!(r_nat.items.iter().all(|s| s.id != "2"));

    // Prefix negation -vocal
    let r_prefix = search(
        vec![s1.clone(), s2.clone(), s3.clone()],
        &SearchQuery { text: "rain -vocal".into(), ..Default::default() },
        &["src_1".into()],
    ).unwrap();
    assert_eq!(r_prefix.total, 2);
    assert!(r_prefix.items.iter().all(|s| s.id != "2"));

    // Quoted phrase negation
    let r_quoted_neg = search(
        vec![s1.clone(), s2.clone(), s3.clone()],
        &SearchQuery { text: r#"rain -"with thunder""#.into(), ..Default::default() },
        &["src_1".into()],
    ).unwrap();
    assert_eq!(r_quoted_neg.total, 2);
    assert!(r_quoted_neg.items.iter().all(|s| s.id != "3"));
}

#[test]
fn typo_corpus_and_transposition() {
    let s1 = make_sound("1", "Scratch Effect", 1.0, vec![], "");
    let s2 = make_sound("2", "Whoosh Fast", 1.0, vec![], "");
    let s3 = make_sound("3", "Laser Beam", 1.0, vec![], "");
    let s4 = make_sound("4", "Raindrop", 1.0, vec![], "");
    let s5 = make_sound("5", "Train Engine", 1.0, vec![], "");

    let sounds = vec![s1, s2, s3, s4, s5];

    // Transposition: scartch -> scratch
    let r1 = search(sounds.clone(), &SearchQuery { text: "scartch".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r1.total, 1);
    assert_eq!(r1.items[0].title, "Scratch Effect");
    assert!(r1.interpretation.corrected.iter().any(|c| c.contains("scartch -> scratch")));

    // Extra character: whooshh -> whoosh
    let r2 = search(sounds.clone(), &SearchQuery { text: "whooshh".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r2.total, 1);
    assert_eq!(r2.items[0].title, "Whoosh Fast");

    // Omission: lasr -> laser
    let r3 = search(sounds.clone(), &SearchQuery { text: "lasr".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r3.total, 1);
    assert_eq!(r3.items[0].title, "Laser Beam");

    // Real word distinction: train vs rain must not cross-match
    let r_train = search(sounds.clone(), &SearchQuery { text: "train".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r_train.total, 1);
    assert_eq!(r_train.items[0].title, "Train Engine");

    let r_rain = search(sounds, &SearchQuery { text: "rain".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r_rain.total, 1);
    assert_eq!(r_rain.items[0].title, "Raindrop");
}

#[test]
fn unicode_and_accent_matching() {
    let s1 = make_sound("1", "Flûte enchantée", 2.0, vec![], "");
    let s2 = make_sound("2", "Éclat métallique", 2.0, vec![], "");
    let s3 = make_sound("3", "雨の音 Japanese Rain", 2.0, vec![], "");

    let sounds = vec![s1, s2, s3];

    let r1 = search(sounds.clone(), &SearchQuery { text: "flute".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r1.total, 1);
    assert_eq!(r1.items[0].title, "Flûte enchantée");

    let r2 = search(sounds.clone(), &SearchQuery { text: "eclat".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r2.total, 1);
    assert_eq!(r2.items[0].title, "Éclat métallique");

    let r3 = search(sounds, &SearchQuery { text: "雨".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r3.total, 1);
    assert_eq!(r3.items[0].title, "雨の音 Japanese Rain");
}

#[test]
fn strict_duration_boundaries() {
    let s1 = make_sound("1", "Boundary 3s", 3.0, vec![], "");
    let s2 = make_sound("2", "Just Under 3s", 2.999, vec![], "");
    let s3 = make_sound("3", "Boundary 5s", 5.0, vec![], "");
    let s4 = make_sound("4", "Just Over 5s", 5.001, vec![], "");

    let sounds = vec![s1, s2, s3, s4];

    // under 3 seconds: duration >= 3.0 excluded
    let r_under = search(sounds.clone(), &SearchQuery { text: "under 3 seconds".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r_under.total, 1);
    assert_eq!(r_under.items[0].title, "Just Under 3s");

    // over 5 seconds: duration <= 5.0 excluded
    let r_over = search(sounds, &SearchQuery { text: "over 5 seconds".into(), ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(r_over.total, 1);
    assert_eq!(r_over.items[0].title, "Just Over 5s");
}

#[test]
fn deterministic_pagination_stability() {
    let mut sounds = Vec::new();
    for i in 0..25 {
        sounds.push(make_sound(&format!("{i:02}"), &format!("Sound Effect {i:02}"), 1.0, vec![], ""));
    }

    // Full 25 result set
    let full = search(sounds.clone(), &SearchQuery { limit: Some(25), offset: 0, ..Default::default() }, &["src_1".into()]).unwrap();
    assert_eq!(full.total, 25);
    assert_eq!(full.items.len(), 25);

    // Waveforms must be stripped in search items
    for item in &full.items {
        assert!(item.profile.as_ref().unwrap().waveform.is_empty());
    }

    // Paged across 5 pages of 5 items
    let mut paged_ids = Vec::new();
    for page in 0..5 {
        let p = search(sounds.clone(), &SearchQuery { limit: Some(5), offset: page * 5, ..Default::default() }, &["src_1".into()]).unwrap();
        assert_eq!(p.total, 25);
        assert_eq!(p.items.len(), 5);
        paged_ids.extend(p.items.into_iter().map(|s| s.id));
    }

    let full_ids: Vec<String> = full.items.into_iter().map(|s| s.id).collect();
    assert_eq!(paged_ids, full_ids, "Pagination slices must match full sequence exactly");
}

#[test]
fn search_facets_computation() {
    let mut s1 = make_sound("1", "Short Mono Whoosh", 1.0, vec!["whoosh".into(), "transition".into()], "");
    s1.profile = Some(test_profile(1.0, 1, "mono", vec![]));

    let mut s2 = make_sound("2", "Short Stereo Whoosh", 2.5, vec!["whoosh".into()], "");
    s2.profile = Some(test_profile(2.5, 2, "stereo", vec![]));

    let mut s3 = make_sound("3", "Medium Ambience", 10.0, vec!["ambience".into()], "");
    s3.profile = Some(test_profile(10.0, 6, "5.1", vec![]));

    let mut s4 = make_sound("4", "Long Stereo Ambience", 20.0, vec!["ambience".into(), "transition".into()], "");
    s4.profile = Some(test_profile(20.0, 2, "stereo", vec![]));

    let r = search(vec![s1, s2, s3, s4], &SearchQuery::default(), &["src_1".into()]).unwrap();
    assert_eq!(r.total, 4);

    // Check tags facets
    let tag_map: std::collections::HashMap<_, _> = r.facets.tags.into_iter().map(|f| (f.value, f.count)).collect();
    assert_eq!(tag_map.get("whoosh"), Some(&2));
    assert_eq!(tag_map.get("transition"), Some(&2));
    assert_eq!(tag_map.get("ambience"), Some(&2));

    // Check layouts facets
    let layout_map: std::collections::HashMap<_, _> = r.facets.layouts.into_iter().map(|f| (f.value, f.count)).collect();
    assert_eq!(layout_map.get("stereo"), Some(&2));
    assert_eq!(layout_map.get("mono"), Some(&1));
    assert_eq!(layout_map.get("5.1"), Some(&1));

    // Check duration buckets
    let dur_map: std::collections::HashMap<_, _> = r.facets.durations.into_iter().map(|f| (f.value, f.count)).collect();
    assert_eq!(dur_map.get("under 3s"), Some(&2));
    assert_eq!(dur_map.get("3s - 15s"), Some(&1));
    assert_eq!(dur_map.get("over 15s"), Some(&1));
}

fn add_catalog_sound(c: &mut Catalog, source: &Source, root: &Path, rel: &str, _title: &str, duration: f64) -> String {
    let full = root.join(rel);
    fs::write(&full, b"audio data").unwrap();
    let hash = hash_file(&full).unwrap();
    let id = c.register(source, rel, &hash).unwrap();
    c.publish(source, &id, &hash, &test_profile(duration, 2, "stereo", vec![])).unwrap();
    c.annotate(&id, &[], "", false).unwrap();
    id
}

#[test]
fn saved_searches_lifecycle_and_migration() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_saved.db");

    // Lifecycle on modern catalog
    let catalog = Catalog::open(&db_path).unwrap();
    let q1 = SearchQuery { text: "whoosh under 3s".into(), min_duration: None, max_duration: Some(3.0), ..Default::default() };
    let saved1 = catalog.save_search("Short Whooshes", &q1).unwrap();
    assert_eq!(saved1.name, "Short Whooshes");
    assert_eq!(saved1.query, q1);

    let q2 = SearchQuery { text: "rain".into(), ..Default::default() };
    let saved2 = catalog.save_search("Rain Ambience", &q2).unwrap();
    assert_eq!(saved2.name, "Rain Ambience");

    let list = catalog.saved_searches().unwrap();
    assert_eq!(list.len(), 2);
    // Ordered by created_at desc
    assert_eq!(list[0].id, saved2.id);
    assert_eq!(list[1].id, saved1.id);

    // Delete saved search
    catalog.delete_saved_search(&saved1.id).unwrap();
    let remaining = catalog.saved_searches().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, saved2.id);

    drop(catalog);

    // Verify reopening retains saved searches
    let catalog = Catalog::open(&db_path).unwrap();
    let retained = catalog.saved_searches().unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].name, "Rain Ambience");

    // Migration test: create a v2 schema database manually
    let v2_db_path = dir.path().join("v2.db");
    {
        let conn = rusqlite::Connection::open(&v2_db_path).unwrap();
        // Version 2 includes the durable jobs table and content-addressed analyses.
        let v2=include_str!("fixtures/schema_v4.sql")
            .split("CREATE TABLE saved_searches").next().unwrap();
        conn.execute_batch(&v2).unwrap();
        conn.pragma_update(None,"user_version",2).unwrap();
    }

    // Now open with Catalog::open, which should detect v2, migrate to the current schema, and create saved_searches
    let migrated_catalog = Catalog::open(&v2_db_path).unwrap();
    let saved = migrated_catalog.save_search("Migrated Query", &SearchQuery { text: "impact".into(), ..Default::default() }).unwrap();
    assert_eq!(saved.name, "Migrated Query");
    assert_eq!(migrated_catalog.saved_searches().unwrap().len(), 1);
}

#[test]
fn ui_and_mcp_search_identity() {
    let dir = tempdir().unwrap();
    let media = dir.path().join("media");
    fs::create_dir(&media).unwrap();

    let mut catalog = Catalog::open(Path::new(":memory:")).unwrap();
    let src = catalog.add_source(&media).unwrap();

    add_catalog_sound(&mut catalog, &src, &media, "laser.wav", "laser.wav", 1.2);
    add_catalog_sound(&mut catalog, &src, &media, "whoosh.wav", "whoosh.wav", 2.0);

    let query = SearchQuery { text: "laser".into(), ..Default::default() };

    // Canonical search called by UI and MCP
    let res1 = catalog.search(&query).unwrap();
    let res2 = catalog.search(&query).unwrap();

    assert_eq!(res1, res2, "Multiple calls from UI or MCP must produce identical SearchResults");
    assert_eq!(res1.total, 1);
    assert_eq!(res1.items[0].relative_path, "laser.wav");
}

#[test]
fn scale_100k_performance() {
    // Generate 100k sounds in memory
    let mut corpus = Vec::with_capacity(100_000);
    let sample_tags = vec![
        vec!["whoosh".into(), "air".into()],
        vec!["impact".into(), "metal".into()],
        vec!["footstep".into(), "dirt".into()],
        vec!["laser".into(), "scifi".into()],
        vec!["rain".into(), "water".into()],
    ];

    for i in 0..100_000 {
        let tag_set = sample_tags[i % sample_tags.len()].clone();
        let dur = ((i % 100) as f64) * 0.1 + 0.5; // 0.5s to 10.4s
        let layout = if i % 2 == 0 { "stereo" } else { "mono" };
        let s = Sound {
            id: format!("snd_{i:06}"),
            source_id: "src_1".into(),
            relative_path: format!("fx/sound_{i:06}.wav"),
            title: format!("Sound Effect {i:06}"),
            content_hash: format!("hash_{i:06}"),
            status: "ready".into(),
            profile: Some(test_profile(dur, if layout == "stereo" { 2 } else { 1 }, layout, vec![])),
            user_tags: tag_set,
            comment: String::new(),
            favorite: i % 50 == 0,
        };
        corpus.push(s);
    }

    let query = SearchQuery {
        text: "whoosh under 3 seconds".into(),
        limit: Some(50),
        offset: 0,
        ..Default::default()
    };

    let start = Instant::now();
    let results = search(corpus, &query, &["src_1".into()]).unwrap();
    let elapsed = start.elapsed();

    assert!(results.total > 0, "Expected matching results in 100k corpus");
    assert_eq!(results.items.len(), 50);
    let budget_ms = if cfg!(debug_assertions) { 600 } else { 100 };
    println!("100k scale search elapsed: {:?} (budget: {}ms)", elapsed, budget_ms);
    assert!(elapsed.as_millis() < budget_ms, "100k search must complete well within performance budget, took {:?}", elapsed);
}

#[test]
fn tauri_camel_case_search_query_deserialization_and_filtering() {
    let json_payload = r#"{
        "text": "whoosh",
        "sourceIds": ["src_1"],
        "favoritesOnly": true,
        "minDuration": 0.5,
        "maxDuration": 5.0,
        "offset": 0,
        "limit": 10
    }"#;

    let query: SearchQuery = serde_json::from_str(json_payload).expect("Must deserialize camelCase SearchQuery from Tauri IPC");
    assert_eq!(query.text, "whoosh");
    assert_eq!(query.source_ids, vec!["src_1"]);
    assert_eq!(query.favorites_only, true);
    assert_eq!(query.min_duration, Some(0.5));
    assert_eq!(query.max_duration, Some(5.0));

    let mut s1 = make_sound("1", "Whoosh In Source 1", 1.0, vec![], "");
    s1.source_id = "src_1".into();
    s1.favorite = true;

    let mut s2 = make_sound("2", "Whoosh In Source 2", 1.0, vec![], "");
    s2.source_id = "src_2".into();
    s2.favorite = true;

    let results = search(vec![s1, s2], &query, &["src_1".into(), "src_2".into()]).unwrap();
    assert_eq!(results.total, 1);
    assert_eq!(results.items[0].source_id, "src_1", "Folder filter must isolate to source_ids");
}

#[test]
fn search_keeps_full_profiles_out_of_results_and_tracks_profile_changes() {
    let root = tempdir().unwrap();
    let mut catalog = Catalog::open(&root.path().join("catalog.sqlite")).unwrap();
    let source = catalog.add_source(root.path()).unwrap();
    let id = catalog.register(&source, "rain.wav", "rain-hash").unwrap();
    let mut profile = test_profile(1.0, 1, "mono", vec!["rain".into()]);
    profile.waveform = vec![[-0.5, 0.5]; 3200];
    catalog.publish(&source, &id, "rain-hash", &profile).unwrap();
    catalog.annotate(&id, &["outdoors".into()], "field recording", true).unwrap();
    let query = SearchQuery { text: "rain".into(), ..Default::default() };
    let result = catalog.search(&query).unwrap();
    assert_eq!(result.total, 1);
    assert!(result.items[0].profile.as_ref().unwrap().waveform.is_empty());
    assert_eq!(result.items[0].comment, "field recording");
    assert_eq!(catalog.sound(&id).unwrap().profile.unwrap().waveform.len(), 3200);
    assert_eq!(catalog.cached_profile("rain-hash").unwrap().unwrap().waveform.len(), 3200);
    let (full, compact): (i64, i64) = catalog.db_connection().query_row(
        "SELECT length(a.profile),length(s.profile) FROM analyses a JOIN search_profiles s USING(content_hash,analyzer)", [], |r| Ok((r.get(0)?,r.get(1)?))
    ).unwrap();
    assert!(compact * 20 < full, "search metadata should exclude waveform bytes");
    profile.tags = vec!["thunder".into()];
    catalog.publish(&source, &id, "rain-hash", &profile).unwrap();
    assert_eq!(catalog.search(&SearchQuery { text: "thunder".into(), ..Default::default() }).unwrap().total, 1);
    catalog.db_connection().execute("DELETE FROM analyses", []).unwrap();
    let count: i64 = catalog.db_connection().query_row("SELECT count(*) FROM search_profiles", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn v8_upgrade_preserves_full_profiles_annotations_and_original_backup() {
    let root = tempdir().unwrap();
    let path = root.path().join("v8.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let source = catalog.add_source(root.path()).unwrap();
    let id = catalog.register(&source, "rain.wav", "rain-hash").unwrap();
    let mut profile = test_profile(1.0, 1, "mono", vec!["rain".into()]);
    profile.waveform = vec![[-0.5, 0.5]; 3200];
    catalog.publish(&source, &id, "rain-hash", &profile).unwrap();
    catalog.annotate(&id, &["custom".into()], "preserved", true).unwrap();
    catalog.db_connection().execute_batch("DROP TRIGGER analyses_search_insert; DROP TRIGGER analyses_search_update; DROP TABLE search_profiles; PRAGMA user_version=8;").unwrap();
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.sound(&id).unwrap().profile.unwrap().waveform.len(), 3200);
    let result = catalog.search(&SearchQuery { text: "rain".into(), ..Default::default() }).unwrap();
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].comment, "preserved");
    assert!(result.items[0].favorite);
    assert!(result.items[0].profile.as_ref().unwrap().waveform.is_empty());
    let backup = fs::read_dir(root.path()).unwrap().map(|e|e.unwrap().path()).find(|p|p.to_string_lossy().contains("pre-v9")).unwrap();
    let old = rusqlite::Connection::open(backup).unwrap();
    assert_eq!(old.pragma_query_value(None,"user_version",|r|r.get::<_,u32>(0)).unwrap(),8);
    assert_eq!(old.query_row("SELECT comment FROM annotations",[],|r|r.get::<_,String>(0)).unwrap(),"preserved");
}

#[test]
fn v8_search_migration_failure_keeps_original_version_and_data() {
    let root = tempdir().unwrap();
    let path = root.path().join("v8.sqlite");
    let catalog = Catalog::open(&path).unwrap();
    let source = catalog.add_source(root.path()).unwrap();
    let id = catalog.register(&source, "rain.wav", "hash").unwrap();
    catalog.annotate(&id, &[], "keep", true).unwrap();
    catalog.db_connection().execute_batch("DROP TRIGGER analyses_search_insert; DROP TRIGGER analyses_search_update; DROP TABLE search_profiles; CREATE TABLE search_profiles(collision TEXT); PRAGMA user_version=8;").unwrap();
    drop(catalog);
    assert!(Catalog::open(&path).is_err());
    let original = rusqlite::Connection::open(path).unwrap();
    assert_eq!(original.pragma_query_value(None,"user_version",|r|r.get::<_,u32>(0)).unwrap(),8);
    assert_eq!(original.query_row("SELECT comment FROM annotations",[],|r|r.get::<_,String>(0)).unwrap(),"keep");
    assert_eq!(original.query_row("SELECT count(*) FROM sqlite_master WHERE name='analyses_search_insert'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}
