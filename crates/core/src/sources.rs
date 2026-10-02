//! Explicit source scopes shared by desktop and future MCP callers.
use crate::{
    catalog::{contained, hash_file, path_text, valid_relative, Catalog, Source},
    invalid, Result,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceFile {
    pub relative_path: String,
    pub sound_id: Option<String>,
    pub status: String,
}

/// Capture catalog identity under its lock, then hash outside the lock.
pub struct FileRelink {
    sound_id: String,
    source_id: String,
    generation: i64,
    content_hash: String,
    parent: PathBuf,
    relative: String,
}
pub struct VerifiedFileRelink(FileRelink);
impl FileRelink {
    pub fn verify(self) -> Result<VerifiedFileRelink> {
        let path = contained(&self.parent, &self.relative)?;
        if hash_file(&path)? != self.content_hash {
            return Err(invalid(
                "Replacement content differs; no relink changes made",
            ));
        }
        Ok(VerifiedFileRelink(self))
    }
}
impl Catalog {
    pub fn source_files(&self, id: &str, online: bool) -> Result<Vec<SourceFile>> {
        let mut q = self.db.prepare("SELECT f.relative_path,s.id,COALESCE(s.status,f.status) FROM source_files f LEFT JOIN sounds s ON s.source_id=f.source_id AND s.relative_path=f.relative_path WHERE f.source_id=?1 ORDER BY f.relative_path")?;
        let rows = q.query_map([id], |r| {
            Ok(SourceFile {
                relative_path: r.get(0)?,
                sound_id: r.get(1)?,
                status: if online { r.get(2)? } else { "offline".into() },
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn set_entry_status(&self, id: &str, relative: &str, status: &str) -> Result<()> {
        self.db.execute(
            "UPDATE source_files SET status=?3 WHERE source_id=?1 AND relative_path=?2",
            params![id, relative, status],
        )?;
        Ok(())
    }
    pub fn validate_source_entry(&self, source_id: &str, relative: &str) -> Result<()> {
        valid_relative(relative)?;
        let allowed: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM sources s WHERE s.id=?1 AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=s.id AND f.relative_path=?2)))", params![source_id,relative], |r|r.get(0))?;
        if !allowed {
            return Err(invalid("File is outside the selected source scope"));
        }
        Ok(())
    }
    /// Select only this file. Resolve against its lexical parent first so an
    /// escaped symlink cannot silently grant access to a different directory.
    pub fn select_file(&self, path: &Path) -> Result<(Source, String)> {
        if path.extension().is_some() && !crate::library::supported(path) {
            return Err(invalid("Unsupported audio extension"));
        }
        if !path.is_absolute() || !path.is_file() {
            return Err(invalid("Choose an existing absolute audio file"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("File has no parent"))?
            .canonicalize()?;
        let name = path_text(Path::new(
            path.file_name()
                .ok_or_else(|| invalid("File has no name"))?,
        ))?;
        let selected = contained(&parent, &name)?;
        let headers = self.source_headers()?;
        // Prefer an existing entry (or recursive source) before reusing a parent
        // file scope, so nested selections retain their original identities.
        let existing = headers.iter().find(|s| {
            selected
                .strip_prefix(&s.root)
                .ok()
                .and_then(|p| path_text(p).ok())
                .is_some_and(|p| {
                    self.validate_source_entry(&s.id, &p.replace(std::path::MAIN_SEPARATOR, "/"))
                        .is_ok()
                })
        });
        let parent_scope = headers
            .iter()
            .filter(|s| parent.starts_with(&s.root))
            .max_by_key(|s| s.root.len());
        let source = if let Some(source) = existing.or(parent_scope) {
            source.clone()
        } else {
            let id = Uuid::new_v4().to_string();
            self.db.execute(
                "INSERT INTO sources(id,name,root,scope) VALUES(?1,?2,?3,'files')",
                params![
                    id,
                    parent.file_name().unwrap_or_default().to_string_lossy(),
                    path_text(&parent)?
                ],
            )?;
            self.source(&id)?
        };
        let relative = path_text(
            selected
                .strip_prefix(&source.root)
                .map_err(|_| invalid("File outside source"))?,
        )?
        .replace(std::path::MAIN_SEPARATOR, "/");
        valid_relative(&relative)?;
        self.db.execute("INSERT INTO source_files(source_id,relative_path) VALUES(?1,?2) ON CONFLICT DO NOTHING",params![source.id,relative])?;
        Ok((self.source(&source.id)?, relative))
    }
    pub fn convert_source_to_folder(&self, id: &str, confirmed: bool) -> Result<Source> {
        if !confirmed {
            return Err(invalid(
                "Confirmation required: conversion imports all supported sibling files recursively",
            ));
        }
        let source = self.source(id)?;
        if source.scope != "files" {
            return Err(invalid("Source already imports a recursive folder"));
        }
        if !Path::new(&source.root).is_dir() {
            return Err(invalid("Source folder is offline"));
        }
        if self.source_headers()?.iter().any(|s| {
            s.id != id
                && (Path::new(&s.root).starts_with(&source.root)
                    || Path::new(&source.root).starts_with(&s.root))
        }) {
            return Err(invalid(
                "Recursive conversion overlaps another source; relink or remove that source first",
            ));
        }
        self.db.execute(
            "UPDATE sources SET scope='folder',generation=generation+1 WHERE id=?1",
            [id],
        )?;
        self.source(id)
    }
    pub fn remove_source(&mut self, id: &str, confirmed: bool) -> Result<()> {
        if !confirmed {
            return Err(invalid("Confirmation required: removal deletes catalog annotations and clip recipes, never original media"));
        }
        self.source(id)?;
        let tx = self.db.transaction()?;
        tx.execute(
            "DELETE FROM clips WHERE sound_id IN(SELECT id FROM sounds WHERE source_id=?1)",
            [id],
        )?;
        tx.execute(
            "DELETE FROM annotations WHERE sound_id IN(SELECT id FROM sounds WHERE source_id=?1)",
            [id],
        )?;
        tx.execute("DELETE FROM sounds WHERE source_id=?1", [id])?;
        tx.execute("DELETE FROM jobs WHERE source_id=?1", [id])?;
        tx.execute("DELETE FROM sources WHERE id=?1", [id])?;
        tx.commit()?;
        Ok(())
    }
    /// Relink one selected sound, including a renamed file, without decoding.
    pub fn relink_file(&mut self, sound_id: &str, path: &Path) -> Result<Source> {
        let verified = self.prepare_file_relink(sound_id, path)?.verify()?;
        self.commit_file_relink(verified)
    }
    pub fn prepare_file_relink(&self, sound_id: &str, path: &Path) -> Result<FileRelink> {
        let sound = self.sound(sound_id)?;
        let source = self.source(&sound.source_id)?;
        if source.scope != "files" {
            return Err(invalid(
                "Individual relink is for selected-files sources; relink this folder instead",
            ));
        }
        if !path.is_absolute() || !path.is_file() {
            return Err(invalid("Choose an existing replacement file"));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("File has no parent"))?
            .canonicalize()?;
        let relative = path_text(Path::new(
            path.file_name()
                .ok_or_else(|| invalid("File has no name"))?,
        ))?;
        let selected = contained(&parent, &relative)?;
        let relative = path_text(
            selected
                .strip_prefix(&parent)
                .map_err(|_| invalid("File outside parent"))?,
        )?
        .replace(std::path::MAIN_SEPARATOR, "/");
        Ok(FileRelink {
            sound_id: sound_id.into(),
            source_id: source.id,
            generation: source.generation,
            content_hash: sound.content_hash,
            parent,
            relative,
        })
    }
    pub fn commit_file_relink(&mut self, verified: VerifiedFileRelink) -> Result<Source> {
        let plan = verified.0;
        let sound = self.sound(&plan.sound_id)?;
        let source = self.source(&plan.source_id)?;
        if source.generation != plan.generation
            || sound.source_id != source.id
            || sound.content_hash != plan.content_hash
        {
            return Err(invalid("Media or source changed during relink; retry"));
        }
        let sound_id = plan.sound_id.as_str();
        let parent = plan.parent;
        let relative = plan.relative;
        // Use a savepoint because selecting a destination may create a scope.
        self.db.execute_batch("SAVEPOINT relink_file")?;
        let result = (|| -> Result<String> {
            let other_overlap = self.source_headers()?.iter().any(|s| {
                s.id != source.id
                    && (parent.starts_with(&s.root) || Path::new(&s.root).starts_with(&parent))
            });
            let (target, relative) = if parent != Path::new(&source.root)
                && source.files.len() == 1
                && !other_overlap
            {
                self.db.execute(
                    "UPDATE sources SET root=?2,name=?3 WHERE id=?1",
                    params![
                        source.id,
                        path_text(&parent)?,
                        parent.file_name().unwrap_or_default().to_string_lossy()
                    ],
                )?;
                self.db.execute("INSERT INTO source_files(source_id,relative_path) VALUES(?1,?2) ON CONFLICT DO NOTHING",params![source.id,relative])?;
                (self.source(&source.id)?, relative)
            } else {
                self.select_file(&parent.join(&relative))?
            };
            let collision: Option<String> = self
                .db
                .query_row(
                    "SELECT id FROM sounds WHERE source_id=?1 AND relative_path=?2",
                    params![target.id, relative],
                    |r| r.get(0),
                )
                .optional()?;
            if collision.as_ref().is_some_and(|id| id != sound_id) {
                return Err(invalid("Replacement path already belongs to another sound"));
            }
            self.db.execute(
                "UPDATE sources SET generation=generation+1 WHERE id=?1 OR id=?2",
                params![source.id, target.id],
            )?;
            self.db.execute("UPDATE sounds SET source_id=?2,relative_path=?3,status=CASE WHEN status='missing' THEN ?4 ELSE status END WHERE id=?1",params![sound_id,target.id,relative,if self.cached_profile(&sound.content_hash)?.is_some(){"ready"}else{"pending"}])?;
            if source.id != target.id || sound.relative_path != relative {
                self.db.execute(
                    "DELETE FROM source_files WHERE source_id=?1 AND relative_path=?2",
                    params![source.id, sound.relative_path],
                )?;
            }
            self.db
                .execute("UPDATE sources SET available=1 WHERE id=?1", [&target.id])?;
            Ok(target.id)
        })();
        match result {
            Ok(id) => {
                self.db.execute_batch("RELEASE relink_file")?;
                self.source(&id)
            }
            Err(e) => {
                self.db
                    .execute_batch("ROLLBACK TO relink_file; RELEASE relink_file")?;
                Err(e)
            }
        }
    }
    pub fn reconcile_paths(
        &mut self,
        source: &Source,
        seen: &[String],
        targets: Option<&[String]>,
        complete: bool,
    ) -> Result<()> {
        if !complete {
            return Ok(());
        }
        if self.source_generation(&source.id)? != source.generation {
            return Err(invalid("Source moved; discard stale scan"));
        }
        let seen: HashSet<&str> = seen.iter().map(String::as_str).collect();
        let targets = targets.map(|t| t.iter().map(String::as_str).collect::<HashSet<_>>());
        let mut q = self.db.prepare(
            "SELECT id,relative_path FROM sounds WHERE source_id=?1 AND status!='missing'",
        )?;
        let existing = q
            .query_map([&source.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(q);
        let tx = self.db.transaction()?;
        for (id, path) in existing {
            if !seen.contains(path.as_str())
                && targets.as_ref().is_none_or(|t| t.contains(path.as_str()))
            {
                tx.execute("UPDATE sounds SET status='missing' WHERE id=?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
