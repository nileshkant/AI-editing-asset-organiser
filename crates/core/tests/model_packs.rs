//! SS-051 lifecycle tests. Transfers use an in-memory fetcher that can
//! truncate, redirect, change identity and drop range support; the production
//! HTTP fetcher is exercised separately against a loopback server.
mod common;
use soundshelf_core::model_pack::*;
use soundshelf_core::model_pack_fetch::*;
use soundshelf_core::model_pack_store::*;
use std::io::Read as _;
use std::{
    collections::{BTreeSet, HashMap},
    io::Cursor,
    sync::{Arc, Mutex},
    time::Duration,
};

const HOST: &str = "models.example.com";
const DRAIN: Duration = Duration::from_millis(200);

fn bytes(seed: u8, len: usize) -> Vec<u8> {
    let mut x = seed as u32 + 1;
    (0..len).map(|_| { x = x.wrapping_mul(1664525).wrapping_add(1013904223); (x >> 24) as u8 }).collect()
}
struct Sys {
    os: &'static str,
    arch: &'static str,
    ram: Option<u64>,
    features: Vec<&'static str>,
    accel: bool,
    runtimes: Vec<&'static str>,
    free: Mutex<Option<u64>>,
}
impl Sys {
    fn with(edit: impl FnOnce(&mut Sys)) -> Arc<Self> {
        let mut system = Arc::try_unwrap(Sys::ok()).ok().unwrap();
        edit(&mut system);
        Arc::new(system)
    }
    fn ok() -> Arc<Self> {
        Arc::new(Self { os: "linux", arch: "x86_64", ram: Some(16 << 30), features: vec!["avx2"], accel: false, runtimes: vec!["onnx"], free: Mutex::new(Some(1 << 40)) })
    }
}
impl System for Sys {
    fn os(&self) -> String { self.os.into() }
    fn arch(&self) -> String { self.arch.into() }
    fn ram_bytes(&self) -> Option<u64> { self.ram }
    fn cpu_features(&self) -> BTreeSet<String> { self.features.iter().map(|s| s.to_string()).collect() }
    fn has_accelerator(&self) -> bool { self.accel }
    fn runtimes(&self) -> BTreeSet<String> { self.runtimes.iter().map(|s| s.to_string()).collect() }
    fn free_disk(&self, _: &std::path::Path) -> Option<u64> { *self.free.lock().unwrap() }
}

fn entry(id: &str, version: &str, capability: Capability, files: &[(&str, &[u8])]) -> PackEntry {
    PackEntry {
        id: id.into(),
        version: version.into(),
        display_name: format!("{id} pack"),
        capability,
        embedding_space: (capability == Capability::ImageEmbedding).then(|| format!("space-{id}")),
        origin: Origin { base_url: format!("https://{HOST}/packs/{id}-{version}/"), hosts: vec![HOST.into(), "cdn.example.com".into()] },
        license: License { spdx: "Apache-2.0".into(), notice: "Test notice".into(), notice_url: Some("https://example.com/notice".into()) },
        requirements: Requirements { os: vec![], arch: vec![], cpu_features: vec![], min_ram_bytes: 1 << 30, needs_accelerator: false, runtime: "onnx".into() },
        files: files.iter().map(|(p, d)| PackFile { path: (*p).into(), bytes: d.len() as u64, blake3: blake3::hash(d).to_hex().to_string() }).collect(),
    }
}
fn manifest(entries: Vec<PackEntry>) -> Manifest {
    let m = Manifest { manifest_version: 1, packs: entries };
    m.validate(Policy::STRICT).unwrap();
    m
}
fn open(root: &std::path::Path, manifest: Manifest, system: Arc<Sys>) -> ModelPacks {
    ModelPacks::open(root.to_path_buf(), manifest, Policy::STRICT, system).unwrap()
}

type Handler = Box<dyn Fn(&url::Url, Option<Range>, Option<&str>) -> Result<Response, PackError> + Send + Sync>;
struct Mock {
    handler: Handler,
    log: Mutex<Vec<(String, Option<Range>)>>,
}
impl Fetcher for Mock {
    fn get(&self, url: &url::Url, range: Option<Range>, if_range: Option<&str>, _: Duration) -> Result<Response, PackError> {
        self.log.lock().unwrap().push((url.to_string(), range));
        (self.handler)(url, range, if_range)
    }
}
fn mock(handler: impl Fn(&url::Url, Option<Range>, Option<&str>) -> Result<Response, PackError> + Send + Sync + 'static) -> Mock {
    Mock { handler: Box::new(handler), log: Mutex::new(vec![]) }
}
fn full(data: &[u8], etag: Option<&str>) -> Response {
    Response { status: 200, headers: Headers { content_length: Some(data.len() as u64), etag: etag.map(str::to_owned), ..Headers::default() }, body: Box::new(Cursor::new(data.to_vec())) }
}
fn partial(data: &[u8], range: Range, etag: Option<&str>) -> Response {
    let last = range.last.min(data.len() as u64 - 1);
    let body = data[range.first as usize..=last as usize].to_vec();
    Response {
        status: 206,
        headers: Headers { content_length: Some(body.len() as u64), content_range: Some((range.first, last, data.len() as u64)), etag: etag.map(str::to_owned), ..Headers::default() },
        body: Box::new(Cursor::new(body)),
    }
}
/// A well-behaved server: serves any file in `files` with range support.
fn server(files: Vec<(String, Vec<u8>)>) -> Mock {
    let files: HashMap<String, Vec<u8>> = files.into_iter().collect();
    mock(move |url, range, _| {
        let name = url.path_segments().unwrap().skip(2).collect::<Vec<_>>().join("/");
        let data = files.get(&name).ok_or_else(|| PackError::Network("404".into()))?;
        Ok(match range {
            Some(r) => partial(data, r, Some("\"v1\"")),
            None => full(data, Some("\"v1\"")),
        })
    })
}
fn served(entry_files: &[(&str, &[u8])]) -> Mock {
    server(entry_files.iter().map(|(p, d)| ((*p).to_owned(), d.to_vec())).collect())
}

// ---------------------------------------------------------------- manifest

#[test]
fn bundled_manifest_is_valid_and_offers_no_unqualified_pack() {
    let manifest = Manifest::parse(BUNDLED_MANIFEST, Policy::STRICT).unwrap();
    assert!(manifest.packs.is_empty(), "no pack may be listed before it passes the SS-050 evaluation");
}

#[test]
fn manifest_rejects_untrusted_or_malformed_entries() {
    let good = entry("clip", "1", Capability::ImageEmbedding, &[("model.onnx", b"abc")]);
    let check = |edit: &dyn Fn(&mut PackEntry)| {
        let mut e = good.clone();
        edit(&mut e);
        Manifest { manifest_version: 1, packs: vec![e] }.validate(Policy::STRICT)
    };
    assert!(check(&|_| {}).is_ok());
    for path in ["../escape.bin", "/abs.bin", "a/../b.bin", "a\\b.bin", ".hidden", "dir//x", "", "C:evil", "a b.bin", "x/"] {
        assert!(check(&|e| e.files[0].path = path.into()).is_err(), "{path:?} must be refused");
    }
    assert!(check(&|e| e.files.push(e.files[0].clone())).is_err(), "duplicate paths");
    assert!(check(&|e| { e.files.push(PackFile { path: "MODEL.ONNX".into(), ..e.files[0].clone() }) }).is_err(), "case-insensitive duplicate");
    assert!(check(&|e| e.files[0].blake3 = "ABC".into()).is_err());
    assert!(check(&|e| e.files[0].bytes = 0).is_err());
    assert!(check(&|e| e.origin.base_url = format!("http://{HOST}/p/")).is_err(), "plain http");
    assert!(check(&|e| e.origin.base_url = format!("https://user:pw@{HOST}/p/")).is_err(), "credentials");
    assert!(check(&|e| e.origin.base_url = format!("https://{HOST}:8443/p/")).is_err(), "odd port");
    assert!(check(&|e| e.origin.base_url = format!("https://{HOST}/p")).is_err(), "no trailing slash");
    assert!(check(&|e| e.origin.hosts = vec!["other.example.com".into()]).is_err(), "origin host must be allowed");
    assert!(check(&|e| e.embedding_space = None).is_err(), "embedding pack must pin its space");
    assert!(check(&|e| e.id = "Bad_Id".into()).is_err());
    assert!(check(&|e| e.version = "1.0-beta".into()).is_err());
    assert!(check(&|e| e.license.notice = " ".into()).is_err(), "a notice must ship with the pack");
    assert!(Manifest { manifest_version: 2, packs: vec![] }.validate(Policy::STRICT).is_err());
    let json = serde_json::to_string(&Manifest { manifest_version: 1, packs: vec![good.clone()] }).unwrap();
    assert!(Manifest::parse(&json.replace("\"runtime\"", "\"extra\":1,\"runtime\""), Policy::STRICT).is_err(), "unknown fields");
    let dup = Manifest { manifest_version: 1, packs: vec![good.clone(), good] };
    assert!(dup.validate(Policy::STRICT).is_err());
}

// ----------------------------------------------------------- compatibility

#[test]
fn incompatible_hosts_are_reported_without_breaking_anything() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = entry("clip", "1", Capability::ImageDescription, &[("m.bin", b"x")]);
    e.requirements.os = vec!["macos".into()];
    let cases: Vec<(Arc<Sys>, IncompatCode, Box<dyn Fn(&mut PackEntry)>)> = vec![
        (Sys::ok(), IncompatCode::UnsupportedOs, Box::new(|_| {})),
        (Sys::with(|s| s.arch = "riscv64"), IncompatCode::UnsupportedArch, Box::new(|e| { e.requirements.os.clear(); e.requirements.arch = vec!["x86_64".into(), "aarch64".into()]; })),
        (Sys::ok(), IncompatCode::MissingCpuFeature, Box::new(|e| { e.requirements.os.clear(); e.requirements.cpu_features = vec!["avx512f".into()]; })),
        (Sys::ok(), IncompatCode::NoAccelerator, Box::new(|e| { e.requirements.os.clear(); e.requirements.needs_accelerator = true; })),
        (Sys::ok(), IncompatCode::RuntimeUnavailable, Box::new(|e| { e.requirements.os.clear(); e.requirements.runtime = "tflite".into(); })),
        (Sys::ok(), IncompatCode::LowMemory, Box::new(|e| { e.requirements.os.clear(); e.requirements.min_ram_bytes = 64 << 30; })),
    ];
    for (system, code, edit) in cases {
        let mut e = e.clone();
        edit(&mut e);
        let packs = open(&dir.path().join(format!("{code:?}")), manifest(vec![e]), system);
        let view = &packs.overview().packs[0];
        assert_eq!(view.phase, Phase::Incompatible, "{code:?}");
        assert_eq!(view.incompatible.as_ref().unwrap().code, code);
        let fetcher = mock(|_, _, _| panic!("an incompatible pack must never be downloaded"));
        assert!(matches!(packs.install("clip", "1", &fetcher), Err(PackError::Incompatible(i)) if i.code == code));
        assert!(packs.overview().active.is_empty());
    }
}

#[test]
fn unreported_memory_is_shown_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let system = Sys::with(|s| s.ram = None);
    let packs = open(dir.path(), manifest(vec![entry("clip", "1", Capability::ImageDescription, &[("m.bin", b"x")])]), system);
    let view = &packs.overview().packs[0];
    assert_eq!(view.phase, Phase::NotInstalled);
    assert!(view.memory_unchecked);
}

// -------------------------------------------------------- install / verify

#[test]
fn install_verifies_every_file_then_activation_reverifies() {
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<(&str, Vec<u8>)> = vec![("model.onnx", bytes(1, 300_000)), ("tokenizer/vocab.txt", bytes(2, 50_000))];
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, d)| (*p, d.as_slice())).collect();
    let e = entry("describe", "1.0", Capability::ImageDescription, &refs);
    let packs = open(dir.path(), manifest(vec![e]), Sys::ok());
    packs.install("describe", "1.0", &served(&refs)).unwrap();
    let view = packs.overview().packs.remove(0);
    assert_eq!(view.phase, Phase::Verified);
    assert_eq!(view.installed_bytes, Some(350_000), "measured footprint is reported");
    assert!(!dir.path().join("staging/describe@1.0").exists(), "staging is gone after commit");
    assert!(dir.path().join("installed/describe/1.0/tokenizer/vocab.txt").is_file());
    let generation = packs.activate("describe", "1.0", DRAIN).unwrap();
    assert_eq!(packs.overview().packs[0].phase, Phase::Active);
    assert_eq!(packs.overview().generation, generation);
    // Corruption after install is caught at activation, not trusted.
    packs.deactivate(Capability::ImageDescription, DRAIN).unwrap();
    let mut model = std::fs::read(dir.path().join("installed/describe/1.0/model.onnx")).unwrap();
    model[10] ^= 0xFF;
    std::fs::write(dir.path().join("installed/describe/1.0/model.onnx"), model).unwrap();
    assert!(matches!(packs.activate("describe", "1.0", DRAIN), Err(PackError::Integrity(_))));
    let view = &packs.overview().packs[0];
    assert_eq!(view.phase, Phase::Failed);
    assert!(packs.overview().active.is_empty());
    assert!(!dir.path().join("installed/describe/1.0").exists(), "a corrupt pack is removed");
}

#[test]
fn wrong_digest_wrong_size_and_overlong_bodies_never_install() {
    let dir = tempfile::tempdir().unwrap();
    let good = bytes(3, 100_000);
    let e = entry("describe", "1", Capability::ImageDescription, &[("m.bin", &good)]);
    let packs = open(dir.path(), manifest(vec![e]), Sys::ok());
    let mut tampered = good.clone();
    tampered[5] ^= 1;
    let corrupt = served(&[("m.bin", &tampered)]);
    assert!(matches!(packs.install("describe", "1", &corrupt), Err(PackError::Integrity(_))));
    assert_eq!(packs.overview().packs[0].phase, Phase::Failed);
    assert_eq!(packs.overview().packs[0].resumable_bytes, 0, "wrong bytes are not kept for resuming");
    // A longer body than the trusted size is refused as it streams.
    let mut longer = good.clone();
    longer.extend_from_slice(b"extra");
    let overlong = mock(move |_, _, _| Ok(full(&longer, None)));
    assert!(matches!(packs.install("describe", "1", &overlong), Err(PackError::Integrity(_))));
    // A server that claims a different size is refused from its headers.
    let short = good[..99_000].to_vec();
    let mismatched = mock(move |_, _, _| Ok(full(&short, None)));
    assert!(packs.install("describe", "1", &mismatched).is_err());
    assert!(packs.overview().active.is_empty());
    assert!(!dir.path().join("installed/describe/1").exists());
    // The right bytes still install afterwards, and the failure clears.
    packs.install("describe", "1", &served(&[("m.bin", &good)])).unwrap();
    assert_eq!(packs.overview().packs[0].phase, Phase::Verified);
}

#[test]
fn truncated_download_resumes_with_a_range_and_verifies_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(4, 20 << 20); // three 8 MiB slices
    let e = entry("describe", "1", Capability::ImageDescription, &[("big.bin", &data)]);
    let packs = open(dir.path(), manifest(vec![e]), Sys::ok());
    // First attempt: the connection dies halfway through the second slice.
    let served_data = data.clone();
    let broken = mock(move |_, range, _| {
        let r = range.unwrap();
        let mut response = partial(&served_data, r, Some("\"v1\""));
        if r.first == 8 << 20 {
            response.body = Box::new(Cursor::new(served_data[r.first as usize..=r.last as usize].to_vec()).take(1 << 20).chain(FailingTail));
        }
        Ok(response)
    });
    assert!(matches!(packs.install("describe", "1", &broken), Err(PackError::Network(_))));
    let view = &packs.overview().packs[0];
    assert_eq!(view.phase, Phase::Failed);
    assert!(view.resumable_bytes >= 9 << 20);
    let kept = std::fs::metadata(dir.path().join("staging/describe@1/big.bin.part")).unwrap().len();
    assert!(kept >= 9 << 20 && kept < 10 << 20, "the downloaded prefix is kept: {kept}");
    // Second attempt resumes exactly where the first stopped.
    let good = served(&[("big.bin", &data)]);
    packs.install("describe", "1", &good).unwrap();
    let log = good.log.lock().unwrap();
    let first_range = log.iter().find_map(|(_, r)| *r).unwrap();
    assert_eq!(first_range.first, kept, "resume starts at the kept length, not from zero");
    assert_eq!(packs.overview().packs[0].phase, Phase::Verified);
    assert_eq!(std::fs::read(dir.path().join("installed/describe/1/big.bin")).unwrap(), data);
}
struct FailingTail;
impl std::io::Read for FailingTail {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset"))
    }
}

#[test]
fn resume_restarts_when_the_file_identity_changed_or_ranges_are_ignored() {
    let data = bytes(5, 18 << 20);
    let e = entry("describe", "1", Capability::ImageDescription, &[("big.bin", &data)]);
    for scenario in ["etag-changed", "range-ignored", "range-not-satisfiable"] {
        let dir = tempfile::tempdir().unwrap();
        let packs = open(dir.path(), manifest(vec![e.clone()]), Sys::ok());
        // Plant a valid-looking interrupted download: the first 9 MiB.
        let served_data = data.clone();
        let interrupted = mock(move |_, range, _| {
            let r = range.unwrap();
            let mut response = partial(&served_data, r, Some("\"v1\""));
            if r.first >= 8 << 20 {
                response.body = Box::new(Cursor::new(served_data[r.first as usize..(r.first as usize + (1 << 20))].to_vec()).chain(FailingTail));
            }
            Ok(response)
        });
        assert!(packs.install("describe", "1", &interrupted).is_err());
        let served_data = data.clone();
        let scenario_name = scenario;
        let tripped = std::sync::atomic::AtomicBool::new(false);
        let second = mock(move |_, range, _| {
            Ok(match (scenario_name, range) {
                ("etag-changed", Some(r)) if r.first > 0 => partial(&served_data, r, Some("\"v2\"")),
                ("etag-changed", Some(r)) => partial(&served_data, r, Some("\"v2\"")),
                ("range-ignored", _) => full(&served_data, None),
                ("range-not-satisfiable", Some(r)) if r.first > 0 && !tripped.swap(true, std::sync::atomic::Ordering::SeqCst) => Response { status: 416, headers: Headers::default(), body: Box::new(Cursor::new(vec![])) },
                (_, Some(r)) => partial(&served_data, r, None),
                (_, None) => full(&served_data, None),
            })
        });
        packs.install("describe", "1", &second).unwrap_or_else(|e| panic!("{scenario}: {e}"));
        assert_eq!(std::fs::read(dir.path().join("installed/describe/1/big.bin")).unwrap(), data, "{scenario}");
    }
}

#[test]
fn redirects_only_go_to_trusted_https_hosts() {
    let data = bytes(6, 10_000);
    let e = entry("describe", "1", Capability::ImageDescription, &[("m.bin", &data)]);
    let moved = |to: &'static str| {
        let data = data.clone();
        mock(move |url, _, _| {
            if url.host_str() == Some(HOST) {
                Ok(Response { status: 302, headers: Headers { location: Some(to.into()), ..Headers::default() }, body: Box::new(Cursor::new(vec![])) })
            } else {
                Ok(full(&data, None))
            }
        })
    };
    let dir = tempfile::tempdir().unwrap();
    let packs = open(dir.path(), manifest(vec![e.clone()]), Sys::ok());
    for bad in ["https://evil.example.net/m.bin", "http://cdn.example.com/m.bin", "https://cdn.example.com.evil.net/m.bin", "https://user@evil.example.net/m.bin"] {
        let result = packs.install("describe", "1", &moved(bad));
        assert!(matches!(result, Err(PackError::RedirectRefused(_))), "{bad}: {result:?}");
    }
    assert!(packs.install("describe", "1", &moved("https://cdn.example.com/m.bin")).is_ok(), "a listed CDN host is allowed");
    // A redirect loop ends instead of spinning.
    let looping = mock(|url, _, _| Ok(Response { status: 301, headers: Headers { location: Some(url.to_string()), ..Headers::default() }, body: Box::new(Cursor::new(vec![])) }));
    let dir2 = tempfile::tempdir().unwrap();
    let packs2 = open(dir2.path(), manifest(vec![e]), Sys::ok());
    assert!(matches!(packs2.install("describe", "1", &looping), Err(PackError::RedirectRefused(_))));
}

#[test]
fn not_enough_disk_is_refused_before_any_download() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(7, 1 << 20);
    let system = Sys::ok();
    let packs = open(dir.path(), manifest(vec![entry("describe", "1", Capability::ImageDescription, &[("m.bin", &data)])]), system.clone());
    *system.free.lock().unwrap() = Some(10 << 20); // below size plus the safety margin
    let fetcher = mock(|_, _, _| panic!("no request without space"));
    assert!(matches!(packs.install("describe", "1", &fetcher), Err(PackError::DiskFull { .. })));
    assert_eq!(packs.overview().packs[0].phase, Phase::Failed);
    *system.free.lock().unwrap() = Some(1 << 40);
    packs.install("describe", "1", &served(&[("m.bin", &data)])).unwrap();
}

#[test]
fn cancellation_keeps_a_resumable_prefix_and_is_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(8, 20 << 20);
    let e = entry("describe", "1", Capability::ImageDescription, &[("big.bin", &data)]);
    let packs = Arc::new(open(dir.path(), manifest(vec![e]), Sys::ok()));
    let hook = packs.clone();
    let served_data = data.clone();
    let cancelling = mock(move |_, range, _| {
        let r = range.unwrap();
        if r.first >= 8 << 20 {
            hook.cancel_install();
        }
        Ok(partial(&served_data, r, Some("\"v1\"")))
    });
    assert!(matches!(packs.install("describe", "1", &cancelling), Err(PackError::Cancelled)));
    let view = &packs.overview().packs[0];
    assert_eq!(view.phase, Phase::NotInstalled, "cancelling is not a failure");
    assert!(view.resumable_bytes >= 8 << 20);
    packs.install("describe", "1", &served(&[("big.bin", &data)])).unwrap();
    assert_eq!(packs.overview().packs[0].phase, Phase::Verified);
    // A discarded partial download frees its space.
    let e2 = entry("describe", "2", Capability::ImageDescription, &[("big.bin", &data)]);
    let dir2 = tempfile::tempdir().unwrap();
    let packs2 = Arc::new(open(dir2.path(), manifest(vec![e2]), Sys::ok()));
    let hook = packs2.clone();
    let served_data = data.clone();
    let cancelling = mock(move |_, range, _| { hook.cancel_install(); Ok(partial(&served_data, range.unwrap(), None)) });
    assert!(packs2.install("describe", "2", &cancelling).is_err());
    assert!(packs2.overview().storage.partial_bytes > 0);
    packs2.discard_partial("describe", "2").unwrap();
    assert_eq!(packs2.overview().storage.partial_bytes, 0);
}

#[test]
fn only_one_install_runs_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(9, 1000);
    let a = entry("a", "1", Capability::ImageDescription, &[("m.bin", &data)]);
    let b = entry("b", "1", Capability::ImageEmbedding, &[("m.bin", &data)]);
    let packs = Arc::new(open(dir.path(), manifest(vec![a, b]), Sys::ok()));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let served_data = data.clone();
    let slow = mock(move |_, _, _| {
        entered_tx.send(()).ok();
        release_rx.lock().unwrap().recv().ok();
        Ok(full(&served_data, None))
    });
    let worker = packs.clone();
    let handle = std::thread::spawn(move || worker.install("a", "1", &slow));
    entered_rx.recv().unwrap();
    let overview = packs.overview();
    assert_eq!(overview.packs[0].phase, Phase::Downloading);
    assert!(matches!(packs.install("b", "1", &mock(|_, _, _| panic!())), Err(PackError::Busy)));
    release_tx.send(()).unwrap();
    handle.join().unwrap().unwrap();
    assert!(packs.install("b", "1", &served(&[("m.bin", &data)])).is_ok());
}

#[test]
fn a_planted_link_where_the_partial_file_goes_is_not_followed() {
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"precious").unwrap();
        let data = bytes(10, 5000);
        let packs = open(&dir.path().join("store"), manifest(vec![entry("a", "1", Capability::ImageDescription, &[("m.bin", &data)])]), Sys::ok());
        let staging = dir.path().join("store/staging/a@1");
        std::fs::create_dir_all(&staging).unwrap();
        std::os::unix::fs::symlink(&victim, staging.join("m.bin.part")).unwrap();
        std::os::unix::fs::symlink(&victim, staging.join("m.bin")).unwrap();
        packs.install("a", "1", &served(&[("m.bin", &data)])).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
        assert_eq!(std::fs::read(dir.path().join("store/installed/a/1/m.bin")).unwrap(), data);
    }
}

// ------------------------------------------------- activation and updates

fn two_versions(space_b: &str) -> (Manifest, Vec<u8>, Vec<u8>) {
    let (d1, d2) = (bytes(11, 40_000), bytes(12, 41_000));
    let v1 = entry("clip", "1.0", Capability::ImageEmbedding, &[("m.bin", &d1)]);
    let mut v2 = entry("clip", "1.1", Capability::ImageEmbedding, &[("m.bin", &d2)]);
    v2.embedding_space = Some(space_b.into());
    (manifest(vec![v1, v2]), d1, d2)
}

#[test]
fn an_update_never_disturbs_the_working_pack_until_it_is_activated_and_can_roll_back() {
    let dir = tempfile::tempdir().unwrap();
    let (m, d1, d2) = two_versions("space-b");
    let packs = open(dir.path(), m, Sys::ok());
    packs.install("clip", "1.0", &served(&[("m.bin", &d1)])).unwrap();
    packs.activate("clip", "1.0", DRAIN).unwrap();
    assert_eq!(packs.overview().packs[0].update_available.as_deref(), Some("1.1"));
    // A failed update leaves version 1.0 active and intact.
    let mut bad = d2.clone();
    bad[0] ^= 1;
    assert!(packs.install("clip", "1.1", &served(&[("m.bin", &bad)])).is_err());
    let overview = packs.overview();
    assert_eq!(overview.active[0].1.version, "1.0");
    assert_eq!(overview.packs[0].phase, Phase::Active);
    assert_eq!(std::fs::read(dir.path().join("installed/clip/1.0/m.bin")).unwrap(), d1);
    // A good update is installed beside it, still not active.
    packs.install("clip", "1.1", &served(&[("m.bin", &d2)])).unwrap();
    assert_eq!(packs.overview().active[0].1.version, "1.0");
    // Activating it changes the vector space, so a reindex is recorded, not mixed.
    let before = packs.generation();
    packs.activate("clip", "1.1", DRAIN).unwrap();
    let overview = packs.overview();
    assert!(overview.generation > before);
    assert_eq!(overview.pending_reindex.len(), 1);
    assert_eq!(overview.pending_reindex[0].from_space.as_deref(), Some("space-clip"));
    assert_eq!(overview.pending_reindex[0].to_space, "space-b");
    // Rolling back is just activating the retained version.
    packs.activate("clip", "1.0", DRAIN).unwrap();
    assert_eq!(packs.overview().active[0].1.version, "1.0");
    let note = packs.overview().pending_reindex[0].clone();
    packs.acknowledge_reindex(Capability::ImageEmbedding, note.generation).unwrap();
}

#[test]
fn packs_with_the_same_pinned_space_switch_without_a_reindex() {
    let dir = tempfile::tempdir().unwrap();
    let (m, d1, d2) = two_versions("space-clip");
    let packs = open(dir.path(), m, Sys::ok());
    packs.install("clip", "1.0", &served(&[("m.bin", &d1)])).unwrap();
    packs.install("clip", "1.1", &served(&[("m.bin", &d2)])).unwrap();
    packs.activate("clip", "1.0", DRAIN).unwrap();
    packs.activate("clip", "1.1", DRAIN).unwrap();
    assert!(packs.overview().pending_reindex.is_empty());
}

#[test]
fn running_tasks_block_replacement_and_removal_and_new_tasks_are_refused_while_draining() {
    let dir = tempfile::tempdir().unwrap();
    let (m, d1, d2) = two_versions("space-b");
    let packs = Arc::new(open(dir.path(), m, Sys::ok()));
    packs.install("clip", "1.0", &served(&[("m.bin", &d1)])).unwrap();
    packs.install("clip", "1.1", &served(&[("m.bin", &d2)])).unwrap();
    packs.activate("clip", "1.0", DRAIN).unwrap();
    let lease = packs.acquire(Capability::ImageEmbedding).unwrap();
    assert_eq!(lease.space_id, "space-clip");
    assert_eq!(packs.overview().packs[0].in_use, 1);
    // Nothing is deleted or switched under a running task.
    assert_eq!(packs.uninstall("clip", "1.0", DRAIN), Err(PackError::InUse(1)));
    assert_eq!(packs.activate("clip", "1.1", DRAIN), Err(PackError::InUse(1)));
    assert!(dir.path().join("installed/clip/1.0/m.bin").is_file());
    assert_eq!(packs.overview().active[0].1.version, "1.0");
    // After a refused drain new tasks may start again.
    drop(packs.acquire(Capability::ImageEmbedding).unwrap());
    // Once the task finishes, removal drains cleanly even while it was waiting.
    let remover = packs.clone();
    let handle = std::thread::spawn(move || remover.uninstall("clip", "1.0", Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(100));
    assert!(matches!(packs.acquire(Capability::ImageEmbedding), Err(PackError::Draining)), "no new task starts while draining");
    drop(lease);
    handle.join().unwrap().unwrap();
    assert!(!dir.path().join("installed/clip/1.0").exists(), "pack-owned files are removed");
    assert!(dir.path().join("installed/clip/1.1/m.bin").is_file(), "other versions are untouched");
    assert!(packs.overview().active.is_empty());
    assert!(matches!(packs.acquire(Capability::ImageEmbedding), Err(PackError::NotActive)));
}

#[test]
fn uninstall_deletes_only_what_the_pack_owns_and_reports_storage() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(13, 70_000);
    let packs = open(dir.path(), manifest(vec![entry("a", "1", Capability::ImageDescription, &[("m.bin", &data)])]), Sys::ok());
    std::fs::write(dir.path().join("unrelated.txt"), b"keep").unwrap();
    packs.install("a", "1", &served(&[("m.bin", &data)])).unwrap();
    assert_eq!(packs.overview().storage.installed_bytes, 70_000);
    packs.activate("a", "1", DRAIN).unwrap();
    packs.uninstall("a", "1", DRAIN).unwrap();
    assert_eq!(packs.overview().storage.installed_bytes, 0);
    assert_eq!(packs.overview().packs[0].phase, Phase::NotInstalled);
    assert!(dir.path().join("unrelated.txt").is_file());
    assert_eq!(packs.uninstall("a", "1", DRAIN), Err(PackError::NotInstalled));
}

// ----------------------------------------------------------- restart rules

#[test]
fn startup_is_offline_and_survives_missing_files_corrupt_state_and_orphans() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(14, 60_000);
    let m = manifest(vec![entry("a", "1", Capability::ImageDescription, &[("m.bin", &data)])]);
    {
        let packs = open(dir.path(), m.clone(), Sys::ok());
        packs.install("a", "1", &served(&[("m.bin", &data)])).unwrap();
        packs.activate("a", "1", DRAIN).unwrap();
    }
    // A clean restart keeps the active pack and performs no request.
    let packs = open(dir.path(), m.clone(), Sys::ok());
    assert_eq!(packs.overview().packs[0].phase, Phase::Active);
    assert!(packs.acquire(Capability::ImageDescription).is_ok());
    drop(packs);
    // A leftover folder from a crash and staging for a pack that left the list are cleaned.
    std::fs::create_dir_all(dir.path().join("installed/ghost/9")).unwrap();
    std::fs::write(dir.path().join("installed/ghost/9/x"), b"x").unwrap();
    std::fs::create_dir_all(dir.path().join("staging/retired@1")).unwrap();
    // A missing file makes the pack fail visibly and deactivates it.
    std::fs::remove_file(dir.path().join("installed/a/1/m.bin")).unwrap();
    let packs = open(dir.path(), m.clone(), Sys::ok());
    let view = &packs.overview().packs[0];
    assert_eq!(view.phase, Phase::Failed);
    assert!(view.detail.as_deref().unwrap().contains("missing"));
    assert!(packs.overview().active.is_empty());
    assert!(!dir.path().join("installed/ghost").exists());
    assert!(!dir.path().join("staging/retired@1").exists());
    // It can be installed again.
    packs.install("a", "1", &served(&[("m.bin", &data)])).unwrap();
    drop(packs);
    // Corrupt state is set aside and the store opens empty instead of failing.
    std::fs::write(dir.path().join("state.json"), b"{not json").unwrap();
    let packs = open(dir.path(), m, Sys::ok());
    assert_eq!(packs.overview().packs[0].phase, Phase::NotInstalled);
    assert!(dir.path().join("state.json.unreadable").is_file());
    assert!(!dir.path().join("installed/a").exists(), "the unknown folder is cleaned");
}

#[test]
fn a_pack_dropped_from_the_trusted_list_can_be_removed_but_not_activated() {
    let dir = tempfile::tempdir().unwrap();
    let data = bytes(15, 20_000);
    let e = entry("a", "1", Capability::ImageDescription, &[("m.bin", &data)]);
    {
        let packs = open(dir.path(), manifest(vec![e]), Sys::ok());
        packs.install("a", "1", &served(&[("m.bin", &data)])).unwrap();
    }
    let packs = open(dir.path(), manifest(vec![]), Sys::ok());
    let view = &packs.overview().packs[0];
    assert!(view.retired);
    assert_eq!(view.phase, Phase::Verified);
    assert!(matches!(packs.activate("a", "1", DRAIN), Err(PackError::Incompatible(i)) if i.code == IncompatCode::RetiredFromManifest));
    packs.uninstall("a", "1", DRAIN).unwrap();
    assert!(packs.overview().packs.is_empty());
}

#[test]
fn the_base_app_needs_no_pack() {
    // With the shipped, empty manifest nothing is installed or active and nothing is requested.
    let dir = tempfile::tempdir().unwrap();
    let packs = ModelPacks::open(dir.path().to_path_buf(), Manifest::parse(BUNDLED_MANIFEST, Policy::STRICT).unwrap(), Policy::STRICT, Arc::new(OsSystem)).unwrap();
    let overview = packs.overview();
    assert!(overview.packs.is_empty() && overview.active.is_empty() && !overview.runtime_available);
    assert!(matches!(packs.acquire(Capability::ImageDescription), Err(PackError::NotActive)));
}

// ------------------------------------------------------- production fetcher

#[test]
fn the_http_fetcher_resumes_with_ranges_and_never_follows_redirects_itself() {
    let data = bytes(16, 300_000);
    let served_data = data.clone();
    let server = common::http::Server::start(Arc::new(move |seen| {
        if seen.path.starts_with("/moved") {
            return (302, vec![("location".into(), "https://evil.example.net/x".into())], vec![]);
        }
        match seen.headers.get("range").and_then(|r| r.strip_prefix("bytes=")) {
            Some(span) => {
                let (a, b) = span.split_once('-').unwrap();
                let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
                let b = b.min(served_data.len() - 1);
                (206, vec![("content-range".into(), format!("bytes {a}-{b}/{}", served_data.len())), ("etag".into(), "\"e1\"".into())], served_data[a..=b].to_vec())
            }
            None => (200, vec![("etag".into(), "\"e1\"".into())], served_data.clone()),
        }
    }));
    let fetcher = HttpFetcher::new(true);
    let url = url::Url::parse(&server.url("/f.bin")).unwrap();
    let mut response = fetcher.get(&url, Some(Range { first: 100, last: 199 }), None, Duration::from_secs(10)).unwrap();
    assert_eq!(response.status, 206);
    assert_eq!(response.headers.content_range, Some((100, 199, 300_000)));
    assert_eq!(response.headers.etag.as_deref(), Some("\"e1\""));
    let mut body = vec![];
    std::io::Read::read_to_end(&mut response.body, &mut body).unwrap();
    assert_eq!(body, data[100..200]);
    let seen = server.requests();
    assert_eq!(seen[0].headers.get("range").map(String::as_str), Some("bytes=100-199"));
    assert_eq!(seen[0].headers.get("accept-encoding").map(String::as_str), Some("identity"));
    // The redirect is reported to the caller, which applies the allowlist.
    let moved = fetcher.get(&url::Url::parse(&server.url("/moved")).unwrap(), None, None, Duration::from_secs(10)).unwrap();
    assert_eq!(moved.status, 302);
    assert_eq!(moved.headers.location.as_deref(), Some("https://evil.example.net/x"));
    // Without the local opt-in, a loopback address is refused at connection time.
    assert!(HttpFetcher::new(false).get(&url, None, None, Duration::from_secs(5)).is_err());
}

#[test]
fn a_full_install_over_the_real_fetcher_to_a_loopback_origin() {
    let data = bytes(17, 150_000);
    let served_data = data.clone();
    let server = common::http::Server::start(Arc::new(move |seen| match seen.headers.get("range").and_then(|r| r.strip_prefix("bytes=")) {
        Some(span) => {
            let (a, b) = span.split_once('-').unwrap();
            let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
            let b = b.min(served_data.len() - 1);
            (206, vec![("content-range".into(), format!("bytes {a}-{b}/{}", served_data.len()))], served_data[a..=b].to_vec())
        }
        None => (200, vec![], served_data.clone()),
    }));
    let mut e = entry("a", "1", Capability::ImageDescription, &[("m.bin", &data)]);
    let host = server.address.ip().to_string();
    e.origin = Origin { base_url: format!("http://{host}:{}/p/", server.address.port()), hosts: vec![host] };
    let policy = Policy { allow_loopback_http: true };
    let m = Manifest { manifest_version: 1, packs: vec![e] };
    m.validate(policy).unwrap();
    assert!(m.validate(Policy::STRICT).is_err(), "production policy refuses plain HTTP");
    let dir = tempfile::tempdir().unwrap();
    let packs = ModelPacks::open(dir.path().to_path_buf(), m, policy, Sys::ok()).unwrap();
    packs.install("a", "1", &HttpFetcher::new(true)).unwrap();
    assert_eq!(std::fs::read(dir.path().join("installed/a/1/m.bin")).unwrap(), data);
}
