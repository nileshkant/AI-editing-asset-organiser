use soundshelf_core::{
    catalog::{Catalog, Profile},
    search::SearchQuery,
};
use std::time::Instant;
#[test]
#[ignore = "100k qualification benchmark; run optimized and capture output"]
fn catalog_search_100k_records() {
    let temp = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open(&temp.path().join("benchmark.sqlite")).unwrap();
    let source = catalog.add_source(temp.path()).unwrap();
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
        tags: vec!["mono".into()],
        waveform: vec![[0.0, 0.5]],
    };
    let start = Instant::now();
    let tx = catalog.db_connection_mut().transaction().unwrap();
    tx.execute(
        "INSERT INTO analyses(content_hash,analyzer,profile) VALUES(?1,?2,?3)",
        rusqlite::params![
            "fixturehash",
            soundshelf_core::catalog::ANALYZER,
            serde_json::to_string(&profile).unwrap()
        ],
    )
    .unwrap();
    {
        let mut insert=tx.prepare("INSERT INTO sounds(id,source_id,relative_path,title,content_hash,status) VALUES(?1,?2,?3,?4,'fixturehash','ready')").unwrap();
        for i in 0..100_000 {
            insert
                .execute(rusqlite::params![
                    format!("id-{i}"),
                    source.id,
                    format!("{i}.wav"),
                    format!("rain ambience {i}")
                ])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    let seed_ms = start.elapsed().as_millis();
    let mut times = vec![];
    for _ in 0..5 {
        let start = Instant::now();
        let result = catalog
            .search(&SearchQuery {
                text: "rain".into(),
                limit: Some(100),
                ..Default::default()
            })
            .unwrap();
        times.push(start.elapsed().as_millis());
        assert_eq!(result.total, 100_000);
        assert_eq!(result.items.len(), 100);
    }
    let report = serde_json::json!({"records":100000,"waveform_buckets_per_profile":1,"shared_profiles":1,"seed_ms":seed_ms,"search_ms":times,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"mode":if cfg!(debug_assertions){"debug"}else{"release"}});
    println!("QUALIFICATION_REPORT={report}");
    if let Some(path) = std::env::var_os("CREATIVESHELF_BENCHMARK_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    // Correctness and bounded output are gating; hardware-specific timings are evidence.
}
#[test]
fn parser_corpus_never_panics_and_never_returns_unbounded_page() {
    let corpus = [
        "",
        "'",
        "\"",
        "under NaN seconds",
        "雨 🌧️ -rain",
        "a\0b",
        "C:\\audio",
        "$(touch /tmp/not-a-command)",
        "../../../",
        "rain OR 1=1",
        "rain -\"thunder clap\"",
        "over 99999999999999999999999999 minutes",
    ];
    for query in corpus {
        let _ = soundshelf_core::search::interpret(&SearchQuery {
            text: query.into(),
            ..Default::default()
        });
    }
    let text = "x".repeat(100_000);
    assert!(soundshelf_core::search::interpret(&SearchQuery {
        text,
        ..Default::default()
    })
    .is_err());
    for len in [0, 1, 3, 4, 8, 16, 31, 64, 128, 512, 4096] {
        let bytes = (0..len)
            .map(|i| ((i * 73 + 19) % 256) as u8)
            .collect::<Vec<_>>();
        let _ = soundshelf_core::portable::preview(&String::from_utf8_lossy(&bytes), false);
        let _ = soundshelf_core::waveform::WaveformPyramid::from_bytes(&bytes);
    }
}

#[test]
fn forged_waveform_lengths_and_algorithm_versions_fail_before_allocation() {
    use soundshelf_core::waveform::WaveformPyramid;
    let pcm = vec![0u8; 4 * 32];
    let original = WaveformPyramid::build(&mut &pcm[..], 48000, 1, 16)
        .unwrap()
        .to_bytes();
    for count in [u32::MAX, 65537] {
        let mut bytes = original.clone();
        bytes[68..72].copy_from_slice(&count.to_le_bytes());
        let hash = blake3::hash(&bytes[64..]);
        bytes[32..64].copy_from_slice(hash.as_bytes());
        assert!(WaveformPyramid::from_bytes(&bytes).is_err());
    }
    let mut bytes = original.clone();
    bytes[16..18].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(WaveformPyramid::from_bytes(&bytes).is_err());
    let mut bytes = original.clone();
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    assert!(WaveformPyramid::from_bytes(&bytes).is_err());
    let mut bytes = original;
    bytes[72..76].copy_from_slice(&f32::NAN.to_le_bytes());
    let hash = blake3::hash(&bytes[64..]);
    bytes[32..64].copy_from_slice(hash.as_bytes());
    assert!(WaveformPyramid::from_bytes(&bytes).is_err());
}
#[test]
fn long_waveforms_adapt_resolution_with_bounded_buckets() {
    use soundshelf_core::waveform::{WaveformPyramid, MAX_BASE_BUCKETS};
    let frames = (MAX_BASE_BUCKETS + 1) * 16;
    let pcm = vec![0u8; frames * 4];
    let waveform = WaveformPyramid::build(&mut &pcm[..], 48000, 1, 16).unwrap();
    assert_eq!(waveform.total_frames, frames as u64);
    assert_eq!(waveform.base_bucket, 32);
    assert!(waveform.levels[0].channels[0].len() <= MAX_BASE_BUCKETS);
    assert!(WaveformPyramid::from_bytes(&waveform.to_bytes()).is_ok());
}
#[test]
fn oversized_cache_file_fails_without_reading_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("oversized.sswf");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert!(soundshelf_core::waveform::WaveformPyramid::read_from_file(&path).is_err());
}
#[test]
#[ignore = "requires explicit FFmpeg tools and loopback bind"]
fn media_input_allowlist_denies_http_and_playlist_demuxers() {
    use soundshelf_core::media::{analyze, run_stream, MediaTools, LOCAL_INPUT_ARGS};
    use std::{
        io::Read,
        process::Command,
        sync::{atomic::AtomicBool, Arc},
        time::Duration,
    };
    let tools = MediaTools::discover().expect("Explicit FFmpeg tools required");
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    server.set_nonblocking(true).unwrap();
    let url = format!("http://{}/audio.wav", server.local_addr().unwrap());
    let mut command = Command::new(&tools.ffprobe);
    command
        .args(LOCAL_INPUT_ARGS)
        .args(["-v", "error", "-i", &url]);
    assert!(run_stream(
        command,
        Arc::new(AtomicBool::new(false)),
        Duration::from_secs(2),
        |r| {
            let mut bytes = vec![];
            r.take(65536).read_to_end(&mut bytes)?;
            Ok(bytes)
        }
    )
    .is_err());
    assert_eq!(
        server.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let temp = tempfile::tempdir().unwrap();
    for content in [
        format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n{url}\n#EXT-X-ENDLIST\n"),
        format!("ffconcat version 1.0\nfile '{url}'\n"),
    ] {
        let path = temp.path().join("playlist-disguised.wav");
        std::fs::write(&path, content).unwrap();
        assert!(analyze(&tools, &path, Arc::new(AtomicBool::new(false))).is_err());
        assert_eq!(
            server.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}

#[test]
#[ignore = "requires explicit FFmpeg fixture tools"]
fn cold_import_and_metadata_reuse_record_io_and_elapsed_time() {
    use soundshelf_core::{media::MediaTools,library::{scan,import_scan},source_catalog};
    use std::sync::{Arc,Mutex,atomic::AtomicBool};
    let tools=MediaTools::discover().expect("Explicit media tools required");let temp=tempfile::tempdir().unwrap();let path=temp.path().join("tone.wav");
    assert!(std::process::Command::new(&tools.ffmpeg).args(["-v","error","-f","lavfi","-i","sine=frequency=440:duration=0.1","-ar","48000"]).arg(&path).status().unwrap().success());
    let cold=Arc::new(Mutex::new(Catalog::open(std::path::Path::new(":memory:")).unwrap()));let source=cold.lock().unwrap().add_source(temp.path()).unwrap();
    source_catalog::take_import_io();let start=Instant::now();scan(cold,source.clone(),&tools,Arc::new(AtomicBool::new(false)),"cold".into(),|_|{}).unwrap();let cold_ms=start.elapsed().as_millis();let cold_io=source_catalog::take_import_io();
    let warm=Arc::new(Mutex::new(Catalog::open(std::path::Path::new(":memory:")).unwrap()));let start=Instant::now();let source=source_catalog::import_folder(&warm,temp.path()).unwrap();import_scan(warm,source,&tools,Arc::new(AtomicBool::new(false)),"warm".into(),|_|{}).unwrap();let warm_ms=start.elapsed().as_millis();let warm_io=source_catalog::take_import_io();
    assert!(cold_io.walks>0 && cold_io.hashes>0 && cold_io.decodes>0);assert_eq!((warm_io.walks,warm_io.hashes,warm_io.decodes),(0,0,0));
    println!("IMPORT_REPORT={{\"cold_ms\":{cold_ms},\"warm_ms\":{warm_ms},\"cold_walks\":{},\"cold_hashes\":{},\"cold_decodes\":{},\"warm_walks\":0,\"warm_hashes\":0,\"warm_decodes\":0}}",cold_io.walks,cold_io.hashes,cold_io.decodes);
}
