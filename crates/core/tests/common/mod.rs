//! Helpers for building genuine older schemas in migration fixtures only.
#![allow(dead_code)]
pub mod http;
pub fn remove_image_schema_for_legacy_fixture(db: &rusqlite::Connection) {
    let images: i64 = db
        .query_row("SELECT count(*) FROM image_assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        images, 0,
        "Only empty image storage can be removed from legacy test fixtures"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM image_import_jobs", [], |r| r.get::<_,i64>(0)).unwrap(), 0,
        "Only empty image import storage can be removed from legacy fixtures");
    assert_eq!(db.query_row("SELECT (SELECT count(*) FROM image_tag_aliases)+(SELECT count(*) FROM image_annotation_actions)", [], |r| r.get::<_,i64>(0)).unwrap(), 0,
        "Only empty annotation storage can be removed from legacy fixtures");
    db.execute_batch(
        "
      DROP TABLE image_generation_provenance;
      DROP TABLE image_generation_sidecars;
      DROP TABLE image_generation_extractions;
      DROP TABLE asset_file_stats;
      DROP TABLE image_source_monitor;
      DROP TABLE image_tag_aliases;
      DROP TABLE image_annotation_actions;
      DROP TRIGGER image_import_audio_conflict;
      DROP TRIGGER audio_import_image_conflict_insert;
      DROP TRIGGER audio_import_image_conflict_update;
      DROP TABLE image_analysis_warnings;
      DROP TABLE image_source_identity;
      DROP TABLE image_import_items;
      DROP TABLE image_import_jobs;
      DROP TABLE image_import_budget;
      DROP TRIGGER registry_sound_insert;
      DROP TRIGGER registry_sound_update;
      DROP TRIGGER registry_sound_delete;
      DROP TABLE image_asset_versions;
      DROP TABLE image_assets;
      DROP TABLE image_analyses;
      DROP TABLE image_contents;
      DROP TABLE asset_registry;
    ",
    )
    .unwrap();
}
