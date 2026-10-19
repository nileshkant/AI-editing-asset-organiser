//! SS-045 gallery windows for the local desktop owner. Pages are a live
//! observation in a deterministic order (selected sort, then ascending asset ID)
//! so a virtualized grid can fetch any visible window. Metadata only: no paths
//! beyond the catalog's relative paths, no media reads.
use crate::{
    catalog::Catalog,
    image_catalog::{scoped, summary, ImageSummary, SUMMARY_COLUMNS},
    image_contracts::{AssetHeader, ImageProfile, MAX_BUNDLE_BYTES, MAX_PAGE_SIZE},
    invalid, Result,
};
use serde::{Deserialize, Serialize};

/// The catalog admits at most 100,000 images; offsets beyond it are invalid.
pub const MAX_GALLERY_OFFSET: u32 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GallerySort {
    /// Relative path, ASCII case-folded, then exact bytes, then asset ID.
    NameAsc,
    NameDesc,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GalleryPage {
    pub items: Vec<ImageSummary>,
    pub offset: u32,
    /// Matching images when this page was read; may change between pages.
    pub total: u32,
}
/// Inspector facts. Bounded fields only; evidence/recipes stay out of the view.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageDetails {
    pub header: AssetHeader,
    pub profile: Option<ImageProfile>,
    pub source_name: String,
    pub source_available: bool,
    pub tags: Vec<String>,
    pub caption: Option<String>,
    /// Creator notes, plain text (SS-046).
    pub comment: String,
    pub favorite: bool,
    /// 1–5 stars; `None` is unrated (never zero).
    pub rating: Option<u8>,
    pub annotation_revision: u32,
}

impl Catalog {
    /// `None` means every registered source (desktop owner); `Some` is a scope list.
    pub fn image_gallery_page(
        &self,
        source_ids: Option<&[String]>,
        sort: GallerySort,
        offset: u32,
        limit: u32,
    ) -> Result<GalleryPage> {
        if limit == 0 || limit > MAX_PAGE_SIZE || offset > MAX_GALLERY_OFFSET {
            return Err(invalid("Invalid gallery window"));
        }
        let mut values: Vec<rusqlite::types::Value> = vec![];
        let (scope, source_scope) = match source_ids {
            None => (String::new(), String::new()),
            Some(ids) => {
                scoped(ids)?;
                if ids.is_empty() {
                    return Ok(GalleryPage {
                        items: vec![],
                        offset,
                        total: 0,
                    });
                }
                values.extend(ids.iter().cloned().map(Into::into));
                let marks = vec!["?"; ids.len()].join(",");
                (format!("AND r.source_id IN ({marks})"), format!("WHERE s.id IN ({marks})"))
            }
        };
        // Folder membership is a materialized set lookup; file scopes need their exact member.
        // Rows without an image record are dropped by the page join below.
        let from = format!(
            "FROM asset_registry r WHERE r.kind='image' {scope}
            AND (r.source_id IN (SELECT id FROM sources WHERE scope='folder')
              OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path))"
        );
        // Per-source counts use the (source_id, kind, id) index instead of a full scan.
        let total: u32 = self.db.query_row(
            &format!(
                "SELECT coalesce(sum(CASE WHEN s.scope='folder'
                   THEN (SELECT count(*) FROM asset_registry r WHERE r.source_id=s.id AND r.kind='image')
                   ELSE (SELECT count(*) FROM source_files f JOIN asset_registry r
                     ON r.source_id=f.source_id AND r.relative_path=f.relative_path
                     WHERE f.source_id=s.id AND r.kind='image') END),0)
                 FROM sources s {source_scope}"
            ),
            rusqlite::params_from_iter(values.iter()),
            |r| r.get(0),
        )?;
        let direction = match sort {
            GallerySort::NameAsc => "ASC",
            GallerySort::NameDesc => "DESC",
        };
        // Sort only keys, then join the page's rows: materializing every summary
        // (joins + JSON) into the sorter made 100K-image windows ~0.5 s.
        let sql = format!(
            "WITH w AS (SELECT r.id, lower(r.relative_path) AS k1, r.relative_path AS k2 {from}
              ORDER BY k1 {direction}, k2 {direction}, r.id ASC LIMIT ? OFFSET ?)
            SELECT {SUMMARY_COLUMNS} FROM w JOIN asset_registry r ON r.id=w.id
            JOIN image_assets i ON i.id=r.id JOIN sources s ON s.id=r.source_id
            LEFT JOIN image_analyses p ON p.content_hash=i.content_hash AND p.analyzer_version=i.analyzer_version
            ORDER BY w.k1 {direction}, w.k2 {direction}, r.id ASC"
        );
        values.push(i64::from(limit).into());
        values.push(i64::from(offset).into());
        let mut statement = self.db.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(values))?;
        let mut items = vec![];
        let mut bytes = 256;
        while let Some(row) = rows.next()? {
            let item = summary(row)?;
            let size = serde_json::to_vec(&item)?.len() + 1;
            // Never truncate silently: a short page tells the caller where to resume.
            if bytes + size > MAX_BUNDLE_BYTES {
                break;
            }
            bytes += size;
            items.push(item);
        }
        Ok(GalleryPage {
            items,
            offset,
            total,
        })
    }
    pub fn image_details(&self, id: &str) -> Result<ImageDetails> {
        let asset = self.image(id, None)?;
        let source = self.source_header(&asset.header.source_id)?;
        Ok(ImageDetails {
            profile: asset.profile,
            source_name: source.name,
            source_available: source.available,
            tags: asset.annotations.tags,
            caption: asset.annotations.caption,
            comment: asset.annotations.comment,
            favorite: asset.annotations.favorite,
            rating: asset.annotations.rating,
            annotation_revision: asset.annotations.revision,
            header: asset.header,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    /// Insert `n` pending image records directly (synthetic metadata, no media).
    fn bulk(c: &Catalog, source: &str, n: u32, name: &str) {
        c.db.execute(
            "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1),
             ids AS (SELECT i, lower(printf('%08x-%04x-4%03x-8%03x-%012x', abs(random()) % 4294967295, i % 65536, i % 4096, (i / 7) % 4096, i)) AS id FROM n)
             INSERT INTO asset_registry(id,kind,source_id,relative_path) SELECT id,'image',?2,printf(?3,i) FROM ids",
            rusqlite::params![n, source, name],
        )
        .unwrap();
        c.db.execute(
            "INSERT INTO image_assets(id,technical_state,availability,enrichment_state,annotation_revision,annotations)
             SELECT id,'pending','available','disabled',1,'{\"revision\":1,\"tags\":[],\"comment\":\"\",\"favorite\":false,\"rating\":null,\"caption\":null}'
             FROM asset_registry WHERE kind='image' AND source_id=?1 AND id NOT IN (SELECT id FROM image_assets)",
            [source],
        )
        .unwrap();
    }
    fn fixture() -> (tempfile::TempDir, Catalog, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        let sa = c.add_source(&a).unwrap().id;
        let sb = c.add_source(&b).unwrap().id;
        (dir, c, sa, sb)
    }
    fn names(p: &GalleryPage) -> Vec<String> {
        p.items.iter().map(|i| i.header.relative_path.clone()).collect()
    }

    #[test]
    fn windows_are_deterministic_case_folded_and_tie_broken_by_id() {
        let (_d, c, sa, sb) = fixture();
        for (s, path) in [(&sa, "b.png"), (&sa, "A.png"), (&sa, "a.png"), (&sb, "a.png"), (&sa, "c/z.webp")] {
            bulk(&c, s, 1, path);
        }
        let all = c.image_gallery_page(None, GallerySort::NameAsc, 0, 100).unwrap();
        assert_eq!(all.total, 5);
        assert_eq!(names(&all), ["A.png", "a.png", "a.png", "b.png", "c/z.webp"]);
        let twins: Vec<_> = all.items[1..3].iter().map(|i| i.header.id.clone()).collect();
        assert!(twins[0] < twins[1], "ID tie-breaker");
        let desc = c.image_gallery_page(None, GallerySort::NameDesc, 0, 100).unwrap();
        assert_eq!(names(&desc), ["c/z.webp", "b.png", "a.png", "a.png", "A.png"]);
        // Windows concatenate to the full order with no duplicates or gaps.
        let mut stitched = vec![];
        for offset in (0..5).step_by(2) {
            let page = c.image_gallery_page(None, GallerySort::NameAsc, offset, 2).unwrap();
            assert_eq!(page.offset, offset);
            stitched.extend(page.items.into_iter().map(|i| i.header.id));
        }
        let expected: Vec<_> = all.items.iter().map(|i| i.header.id.clone()).collect();
        assert_eq!(stitched, expected);
        let scoped = c
            .image_gallery_page(Some(&[sb.clone()]), GallerySort::NameAsc, 0, 100)
            .unwrap();
        assert_eq!((scoped.total, names(&scoped)), (1, vec!["a.png".to_string()]));
        assert_eq!(c.image_gallery_page(Some(&[]), GallerySort::NameAsc, 0, 10).unwrap().total, 0);
        for (offset, limit) in [(0, 0), (0, 101), (MAX_GALLERY_OFFSET + 1, 10)] {
            assert!(c.image_gallery_page(None, GallerySort::NameAsc, offset, limit).is_err());
        }
        // Audio rows are never part of the image gallery.
        assert!(all.items.iter().all(|i| i.header.kind == crate::image_contracts::AssetKind::Image));
    }

    #[test]
    fn files_scoped_sources_list_only_selected_members_and_details_are_bounded() {
        let (_dir, mut c, sa, sb) = fixture();
        c.db.execute("UPDATE sources SET scope='files' WHERE id=?1", [&sb]).unwrap();
        c.db.execute("INSERT INTO source_files(source_id,relative_path) VALUES(?1,'chosen.png')", [&sb])
            .unwrap();
        let source = c.source(&sb).unwrap();
        let image = c.register_image(&source, "chosen.png", None).unwrap();
        // A registry row outside the file grant (and without an image record) is never listed.
        c.db.execute("INSERT INTO asset_registry(id,kind,source_id,relative_path) VALUES('00000000-0000-4000-8000-0000000000aa','image',?1,'sibling.png')", [&source.id])
            .unwrap();
        bulk(&c, &sa, 3, "folder-%d.png");
        let page = c
            .image_gallery_page(Some(&[source.id.clone()]), GallerySort::NameAsc, 0, 10)
            .unwrap();
        assert_eq!((page.total, names(&page)), (1, vec!["chosen.png".to_string()]));
        let details = c.image_details(&image.header.id).unwrap();
        assert_eq!(details.source_name, source.name);
        assert!(details.profile.is_none() && details.tags.is_empty());
        assert!(c.image_details("00000000-0000-4000-8000-000000000001").is_err());
    }

    fn v11_database(path: &std::path::Path) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        for schema in [
            include_str!("schema.sql"),
            include_str!("source_catalog_schema.sql"),
            include_str!("settings_schema.sql"),
            include_str!("search_profile_schema.sql"),
            include_str!("image_schema.sql"),
            include_str!("image_jobs_schema.sql"),
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 11).unwrap();
        db
    }
    #[test]
    fn genuine_v11_upgrade_adds_the_gallery_index_with_a_backup() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v11.sqlite");
        let mut old = Catalog { db: v11_database(&path) };
        let source = old.add_source(d.path()).unwrap();
        let image = old.register_image(&source, "Old.png", Some(&"a".repeat(64))).unwrap();
        let mut notes = image.annotations.clone();
        notes.tags = vec!["kept".into()];
        let image = old
            .set_image_annotations(&image.header.id, Some(&"a".repeat(64)), &notes)
            .unwrap();
        drop(old);
        let c = Catalog::open(&path).unwrap();
        // Later schema steps may follow v12 in the same upgrade.
        assert_eq!(c.db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), crate::catalog::SCHEMA_VERSION);
        assert_eq!(c.image(&image.header.id, None).unwrap(), image);
        let index: i64 = c
            .db
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name='asset_registry_gallery'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(index, 1);
        assert_eq!(c.image_gallery_page(None, GallerySort::NameAsc, 0, 10).unwrap().total, 1);
        let backup = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(&format!("pre-v{}-", crate::catalog::SCHEMA_VERSION)))
            .unwrap();
        let db = rusqlite::Connection::open(backup).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 11);
        assert_eq!(
            db.query_row("SELECT count(*) FROM sqlite_master WHERE name='asset_registry_gallery'", [], |r| r.get::<_, i64>(0)).unwrap(),
            0
        );
        assert_eq!(Catalog { db }.image(&image.header.id, None).unwrap(), image);
    }
    #[test]
    fn v12_collision_rolls_back_and_keeps_the_v11_catalog_usable() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v11.sqlite");
        let db = v11_database(&path);
        db.execute_batch("CREATE TABLE asset_registry_gallery(collision TEXT); INSERT INTO asset_registry_gallery VALUES('retained');")
            .unwrap();
        drop(db);
        assert!(Catalog::open(&path).is_err());
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 11);
        assert_eq!(
            db.query_row("SELECT collision FROM asset_registry_gallery", [], |r| r.get::<_, String>(0)).unwrap(),
            "retained"
        );
    }

    /// T3 scale check at the catalog's 100,000-image cap.
    #[test]
    fn full_catalog_windows_stay_bounded_at_the_image_cap() {
        let (_d, c, sa, _) = fixture();
        bulk(&c, &sa, 100_000, "library/%06d-長い名前.png");
        let mut worst = std::time::Duration::ZERO;
        for offset in [0, 50_000, 99_900] {
            let start = Instant::now();
            let page = c.image_gallery_page(None, GallerySort::NameAsc, offset, 100).unwrap();
            worst = worst.max(start.elapsed());
            assert_eq!((page.total, page.items.len()), (100_000, 100));
            assert_eq!(page.items[0].header.relative_path, format!("library/{:06}-長い名前.png", offset + 1));
        }
        let mut worst_desc = std::time::Duration::ZERO;
        for offset in [0, 50_000, 99_900] {
            let start = Instant::now();
            let page = c.image_gallery_page(None, GallerySort::NameDesc, offset, 100).unwrap();
            worst_desc = worst_desc.max(start.elapsed());
            assert_eq!(page.items[0].header.relative_path, format!("library/{:06}-長い名前.png", 100_000 - offset));
        }
        eprintln!("SS-045 gallery window at 100K images, descending: worst {worst_desc:?}");
        let end = c.image_gallery_page(None, GallerySort::NameAsc, 99_990, 100).unwrap();
        assert_eq!(end.items.len(), 10);
        eprintln!("SS-045 gallery window at 100K images: worst {worst:?}");
        // The v12 index serves the order directly (no temporary sort B-tree).
        let mut q = c
            .db
            .prepare("EXPLAIN QUERY PLAN SELECT r.id FROM asset_registry r WHERE r.kind='image' ORDER BY lower(r.relative_path), r.relative_path, r.id LIMIT 100 OFFSET 50000")
            .unwrap();
        let plan: Vec<String> = q.query_map([], |r| r.get(3)).unwrap().map(|r| r.unwrap()).collect();
        assert!(plan.iter().any(|p| p.contains("asset_registry_gallery")), "{plan:?}");
        assert!(!plan.iter().any(|p| p.contains("TEMP B-TREE")), "{plan:?}");
    }
}
