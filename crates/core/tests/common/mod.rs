//! Helpers for building genuine older schemas in migration fixtures only.
#![allow(dead_code)]
pub fn remove_image_schema_for_legacy_fixture(db: &rusqlite::Connection) {
    let images: i64 = db
        .query_row("SELECT count(*) FROM image_assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        images, 0,
        "Only empty image storage can be removed from legacy test fixtures"
    );
    db.execute_batch(
        "
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
