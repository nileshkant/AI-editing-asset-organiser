//! Deliberate backups and metadata-only diagnostics; original audio is excluded.
use crate::{
    catalog::{path_text, Catalog, SCHEMA_VERSION},
    invalid, Result,
};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, time::Duration};
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub output_device: Option<String>,
    pub imports_paused: bool,
}
impl Catalog {
    pub fn preferences(&self) -> Result<Preferences> {
        use rusqlite::OptionalExtension;
        let raw: Option<String> = self
            .db
            .query_row(
                "SELECT value FROM app_settings WHERE key='preferences'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match raw {
            Some(raw) => serde_json::from_str(&raw)?,
            None => Preferences::default(),
        })
    }
    pub fn set_preferences(&self, preferences: &Preferences) -> Result<()> {
        if preferences
            .output_device
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
        {
            return Err(invalid("Invalid output device"));
        }
        self.db.execute("INSERT INTO app_settings(key,value) VALUES('preferences',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(preferences)?])?;
        Ok(())
    }
    pub fn backup_database(&self, destination: &Path) -> Result<()> {
        let parent = destination
            .parent()
            .ok_or_else(|| invalid("Choose a backup file"))?;
        if !destination.is_absolute() || destination.exists() {
            return Err(invalid("Choose a new absolute backup path"));
        }
        let stage = parent.join(format!(
            ".creativeshelf-backup-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options.open(&stage)?;
            self.db.execute("VACUUM INTO ?1", [path_text(&stage)?])?;
            // Windows FlushFileBuffers requires a handle with write access.
            fs::OpenOptions::new().read(true).write(true).open(&stage)?.sync_all()?;
            fs::hard_link(&stage, destination)?;
            #[cfg(unix)]
            fs::File::open(parent)?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(stage);
        result
    }
    /// Must run under the catalog lock with import/export work stopped.
    /// Restores only current compatible schema, with a rollback snapshot first.
    pub fn restore_database(&mut self, backup: &Path, rollback: &Path) -> Result<()> {
        let meta = fs::symlink_metadata(backup)?;
        if !meta.is_file() || meta.len() > 512 * 1024 * 1024 {
            return Err(invalid(
                "Backup must be a regular SQLite file up to 512 MiB",
            ));
        }
        let source = Connection::open_with_flags(
            backup,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        source.pragma_update(None, "trusted_schema", false)?;
        let version: u32 = source.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(invalid(
                "Backup version differs; use a matching app or portable catalog import",
            ));
        }
        let integrity: String = source.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" || schema(&source)? != schema(&self.db)? {
            return Err(invalid("Backup integrity or schema mismatch"));
        }
        let violations: i64 =
            source.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })?;
        if violations != 0 {
            return Err(invalid("Backup contains invalid references"));
        }
        let active: i64 = self.db.query_row(
            "SELECT count(*) FROM jobs WHERE state IN ('queued','running')",
            [],
            |r| r.get(0),
        )?;
        if active != 0 {
            return Err(invalid("Finish or cancel imports before restoring"));
        }
        self.backup_database(rollback)?;
        {
            let copy = rusqlite::backup::Backup::new(&source, &mut self.db)?;
            copy.run_to_completion(128, Duration::from_millis(5), None)?;
        }
        self.db.execute("UPDATE jobs SET state='cancelled',lease_owner=NULL,lease_until=NULL,status='Cancelled by database restore' WHERE state IN ('queued','running')",[])?;
        // Retain saved revisions: newer source snapshots cause a visible conflict
        // instead of overwriting post-backup source annotations in the background.
        self.db.execute("UPDATE source_catalog_state SET dirty=dirty+1,last_error='Restored backup: source metadata needs review; newer snapshots are protected by revision checks'",[])?;
        Ok(())
    }
    pub fn support_report(&self, media_available: bool) -> Result<String> {
        let sounds: i64 = self
            .db
            .query_row("SELECT count(*) FROM sounds", [], |r| r.get(0))?;
        let sources: i64 = self
            .db
            .query_row("SELECT count(*) FROM sources", [], |r| r.get(0))?;
        let dirty: i64 = self.db.query_row(
            "SELECT count(*) FROM source_catalog_state WHERE dirty>0",
            [],
            |r| r.get(0),
        )?;
        // Allowlist fields; never serialize paths, labels, comments, errors or tokens.
        Ok(serde_json::to_string_pretty(
            &serde_json::json!({"schema":"creativeshelf-support/v1","app_version":env!("CARGO_PKG_VERSION"),"database_version":SCHEMA_VERSION,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"media_engine_available":media_available,"source_count":sources,"sound_count":sounds,"dirty_source_catalogs":dirty,"telemetry":false,"ai_recognition":false}),
        )?)
    }
}
fn schema(db: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement=db.prepare("SELECT type,name,coalesce(sql,'') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")?;
    let rows = statement.query_map([], |r| {
        let sql: String = r.get(2)?;
        Ok((
            r.get(0)?,
            r.get(1)?,
            sql.chars().filter(|c| !c.is_ascii_whitespace()).collect(),
        ))
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
