use serde_json::{json, Value};
use soundshelf_core::image_contracts::*;
use std::collections::BTreeMap;

const BASE: &str = include_str!("../../../fixtures/contracts/image-v1.json");
const CASES: &str = include_str!("../../../fixtures/contracts/image-v1-cases.json");

fn edited(mut base: Value, edits: &Value) -> Value {
    for edit in edits.as_array().unwrap() {
        let path = edit["path"].as_str().unwrap();
        let (parent, key) = path.rsplit_once('/').unwrap();
        let parent = base.pointer_mut(parent).unwrap();
        if edit["remove"].as_bool() == Some(true) {
            parent.as_object_mut().unwrap().remove(key);
        } else if let Some(array) = parent.as_array_mut() {
            array[key.parse::<usize>().unwrap()] = edit["value"].clone();
        } else {
            parent
                .as_object_mut()
                .unwrap()
                .insert(key.into(), edit["value"].clone());
        }
    }
    base
}
#[test]
fn shared_conformance_corpus_and_lossless_roundtrip() {
    let base: Value = serde_json::from_str(BASE).unwrap();
    let bundle = ContractBundle::parse(BASE.as_bytes()).unwrap();
    assert_eq!(serde_json::to_value(&bundle).unwrap(), base);
    assert_eq!(
        ContractBundle::parse(&serde_json::to_vec(&bundle).unwrap()).unwrap(),
        bundle
    );
    let cases: Value = serde_json::from_str(CASES).unwrap();
    for case in cases.as_array().unwrap() {
        let value = edited(base.clone(), &case["edits"]);
        let result = ContractBundle::parse(&serde_json::to_vec(&value).unwrap());
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        if let Some(code) = case.get("error") {
            assert_eq!(
                serde_json::to_value(result.unwrap_err()).unwrap(),
                *code,
                "{}",
                case["name"]
            );
        }
    }
}
#[test]
fn boundaries_and_nonfinite_values_are_rejected_without_large_allocations() {
    let base = ContractBundle::parse(BASE.as_bytes()).unwrap();
    let mut b = base.clone();
    b.images[0].profile.as_mut().unwrap().raw_width = u32::MAX;
    assert!(b.validate().is_err());
    let mut b = base.clone();
    b.images[0].annotations.tags = vec!["x".into(); MAX_TAGS + 1];
    assert_eq!(b.validate(), Err(ContractError::LimitExceeded));
    let mut b = base.clone();
    b.images[0].annotations.comment = "é".repeat(MAX_TEXT_BYTES / 2 + 1);
    assert_eq!(b.validate(), Err(ContractError::LimitExceeded));
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut b = base.clone();
        b.images[0].evidence[0].confidence = Some(number);
        b.images[0].evidence[0].confidence_kind = ConfidenceKind::Reported;
        assert!(b.validate().is_err());
    }
    assert_eq!(
        ContractBundle::parse(&vec![b' '; MAX_BUNDLE_BYTES + 1]),
        Err(ContractError::LimitExceeded)
    );
    assert!(ContractBundle::parse(b"{invalid").is_err());
    let invalid_unicode = BASE.replace("fixture-image-v1", r"\ud800");
    assert!(ContractBundle::parse(invalid_unicode.as_bytes()).is_err());
}
#[test]
fn ipc_uses_camel_case_and_explicit_revision() {
    let value: Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/image-edit-args.json"
    ))
    .unwrap();
    let args: ImageEditArgs = serde_json::from_value(value.clone()).unwrap();
    args.validate().unwrap();
    assert_eq!(serde_json::to_value(&args).unwrap(), value);
    for edit in [
        json!({"path":"/expectedRevision","remove":true}),
        json!({"path":"/expectedRevision","value":0}),
        json!({"path":"/asset_id","value":args.asset_id}),
    ] {
        let mutated = edited(value.clone(), &json!([edit]));
        assert!(serde_json::from_value::<ImageEditArgs>(mutated)
            .map(|a| a.validate().is_err())
            .unwrap_or(true));
    }
}

#[test]
fn integral_json_number_spellings_match_javascript() {
    let baseline = ContractBundle::parse(BASE.as_bytes()).unwrap();
    for spelling in ["6.0", "6e0"] {
        let wire = BASE.replacen(
            "\"orientation\": 6",
            &format!("\"orientation\": {spelling}"),
            1,
        );
        assert_eq!(ContractBundle::parse(wire.as_bytes()).unwrap(), baseline);
    }
    let fractional = BASE.replacen("\"orientation\": 6", "\"orientation\": 6.5", 1);
    assert!(ContractBundle::parse(fractional.as_bytes()).is_err());
}
#[test]
fn legacy_portable_snapshot_and_agent_payloads_remain_unchanged() {
    use soundshelf_core::{
        agent::{Access, AgentLibrary},
        catalog::{Catalog, SCHEMA_VERSION},
        export::ExportService,
        portable::PortableCatalog,
        source_catalog::Snapshot,
    };
    use std::{
        fs,
        sync::{Arc, Mutex},
    };
    let expected: Value = serde_json::from_str(include_str!(
        "../../../fixtures/contracts/legacy-audio-v1.json"
    ))
    .unwrap();
    let portable: PortableCatalog = serde_json::from_value(expected["portable"].clone()).unwrap();
    portable.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&portable).unwrap(),
        expected["portable"]
    );
    let snapshot: Snapshot = serde_json::from_value(expected["source_snapshot"].clone()).unwrap();
    snapshot.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap(),
        expected["source_snapshot"]
    );
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    fs::create_dir(&media).unwrap();
    let original = b"synthetic fixture";
    fs::write(media.join("音声.wav"), original).unwrap();
    let mut c = Catalog::open(&dir.path().join("catalog.sqlite")).unwrap();
    // The audio wire fixture remains v9-era data after the additive v10 upgrade.
    assert!(SCHEMA_VERSION >= 9);
    let roots = BTreeMap::from([("fixture-source".into(), media.to_str().unwrap().into())]);
    c.import_portable(&portable, &roots, dir.path()).unwrap();
    assert_eq!(
        serde_json::to_value(c.export_portable().unwrap()).unwrap(),
        expected["portable"]
    );
    let library = Arc::new(AgentLibrary::new(
        Arc::new(Mutex::new(c)),
        Arc::new(ExportService::new(dir.path().join("exports")).unwrap()),
        None,
    ));
    let access = Access {
        source_ids: vec!["fixture-source".into()],
        read: true,
        ..Default::default()
    };
    let sound = &portable.sounds[0];
    let clip = &portable.clips[0];
    let calls = [
        (
            "sounds.get",
            json!({"id":sound.id,"version":sound.content_hash}),
            "sounds_get",
        ),
        ("sounds.search", json!({}), "sounds_search"),
        ("clips.get", json!({"id":clip.id}), "clips_get"),
        (
            "clips.list",
            json!({"id":sound.id,"version":sound.content_hash}),
            "clips_list",
        ),
    ];
    for (method, args, snapshot) in calls {
        assert_eq!(
            library
                .execute("fixture-client", &access, method, args)
                .unwrap(),
            expected[snapshot],
            "{method}"
        );
    }
    let image = ContractBundle::parse(BASE.as_bytes())
        .unwrap()
        .images
        .remove(0);
    let wrong = library
        .execute(
            "fixture-client",
            &access,
            "sounds.get",
            json!({"id":image.header.id,"version":image.header.content_hash}),
        )
        .unwrap_err();
    assert_eq!(wrong.code, "PERMISSION_DENIED");
    assert_eq!(
        library
            .execute("fixture-client", &access, "images.search", json!({}))
            .unwrap_err()
            .code,
        "INVALID_ARGUMENT"
    );
    let mut future = expected["portable"].clone();
    future["schema"] = json!("soundshelf-catalog/v2");
    assert!(serde_json::from_value::<PortableCatalog>(future)
        .unwrap()
        .validate()
        .is_err());
    let mut wrong = expected["portable"].clone();
    wrong["images"] = json!([image]);
    assert!(serde_json::from_value::<PortableCatalog>(wrong).is_err());
    assert_eq!(fs::read(media.join("音声.wav")).unwrap(), original);
}
