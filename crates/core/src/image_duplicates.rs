//! SS-062 exact-duplicate stacks. Membership is derived on read from verified
//! content digests, so stacks follow file changes, relinks and restores with no
//! recomputation job. Only a creator's explicit choices persist: the preferred
//! copy, copies kept separate, and dismissal. Grouping never deletes, moves or
//! merges files, paths or per-file annotations.
//!
//! A copy is a member when it is an image (never a derived element), its bytes
//! were hashed and are not awaiting re-verification (`technical_state` is not
//! `pending`), and it is not confirmed missing. Offline and inaccessible copies
//! stay members and report their state. Visual near-duplicates are a separate
//! kind that needs SS-060 retrieval and is not offered here.
use crate::{
    catalog::Catalog,
    image_catalog::{scoped, summary, ImageSummary, SUMMARY_COLUMNS},
    image_contracts::MAX_BUNDLE_BYTES,
    mixed_catalog::PortableImage,
};
use rusqlite::{params, types::Value, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Stacks per page. Each carries at most `MAX_MEMBERS` copies.
pub const MAX_GROUP_PAGE: u32 = 25;
/// Copies listed per stack; `member_count` and `separated_count` stay exact.
pub const MAX_MEMBERS: usize = 64;
/// Same bound as gallery offsets: the catalog holds at most 100,000 images.
pub const MAX_GROUP_OFFSET: u32 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DuplicateError {
    #[error("Invalid duplicate request")]
    InvalidArgument,
    #[error("Duplicate stack or copy not found")]
    NotFound,
    #[error("Copy is outside its approved scope")]
    PermissionDenied,
    #[error("Stack changed elsewhere; refresh and retry")]
    Conflict,
    #[error("Duplicate operation failed")]
    Internal,
}
impl From<rusqlite::Error> for DuplicateError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Internal
    }
}
impl From<crate::Error> for DuplicateError {
    fn from(_: crate::Error) -> Self {
        Self::Internal
    }
}
impl From<serde_json::Error> for DuplicateError {
    fn from(_: serde_json::Error) -> Self {
        Self::Internal
    }
}
type Result<T> = std::result::Result<T, DuplicateError>;
use DuplicateError as E;

/// Stacks are labeled by how they were found. Only identical bytes exist today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Exact,
}
/// What became of the creator's preferred copy. `None` also covers a preferred
/// copy outside the caller's scope, so a restricted file is never revealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferredState {
    None,
    Available,
    Offline,
    Inaccessible,
    /// Confirmed gone from its folder.
    Missing,
    /// Its bytes changed or await re-verification, so it left the stack.
    Changed,
    Separated,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplicateMember {
    pub image: ImageSummary,
    pub source_name: String,
    pub separated: bool,
    /// Present now but not when the stack was dismissed.
    pub new_since_dismissed: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplicateGroup {
    pub kind: GroupKind,
    pub content_hash: String,
    /// 0 until the first decision; pass it back as the expected revision.
    pub revision: u32,
    /// Copies in the stack (not separated) within scope.
    pub member_count: u32,
    pub separated_count: u32,
    /// Stack members first (available, then offline/inaccessible), then separated copies.
    pub members: Vec<DuplicateMember>,
    pub members_truncated: bool,
    pub preferred_id: Option<String>,
    pub preferred_state: PreferredState,
    /// The preferred copy while it is a member, else the first listed member.
    pub representative_id: Option<String>,
    pub dismissed: bool,
    /// Dismissed with no new copy, or fewer than two copies left in the stack.
    pub hidden: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DuplicatePage {
    pub groups: Vec<DuplicateGroup>,
    pub offset: u32,
    /// Stacks listed by this query (open only, or open and hidden).
    pub total: u32,
    pub open_groups: u32,
    /// Copies beyond the first in every open stack.
    pub extra_copies: u32,
    pub hidden_groups: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DuplicateAction {
    Prefer { asset_id: String },
    ClearPreferred,
    /// Keep one copy out of the stack. Its file and annotations are untouched.
    Separate { asset_id: String },
    Rejoin { asset_id: String },
    Dismiss,
    Restore,
}

fn valid_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn scope_values(scope: Option<&[String]>) -> Result<(String, Vec<Value>)> {
    match scope {
        None => Ok((String::new(), vec![])),
        Some(ids) => {
            scoped(ids).map_err(|_| E::InvalidArgument)?;
            let marks = vec!["?"; ids.len().max(1)].join(",");
            let values = if ids.is_empty() { vec![Value::Null] } else { ids.iter().cloned().map(Into::into).collect() };
            Ok((format!("AND r.source_id IN ({marks})"), values))
        }
    }
}
/// Copies that may belong to a stack. Folder membership is a set lookup; file
/// scopes need their exact member, as in the gallery.
const MEMBER_FILTER: &str = "r.kind='image' AND i.content_hash IS NOT NULL AND i.technical_state!='pending'
 AND i.availability!='missing'
 AND (r.source_id IN (SELECT id FROM sources WHERE scope='folder')
  OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path))";

/// Open = at least two copies in the stack and not dismissed-and-unchanged.
/// Digests held by one record are dropped from the covering index before any
/// registry lookup, so the cost follows the number of duplicated records.
fn stacks_cte(scope: &str) -> String {
    format!(
        "WITH c AS (SELECT content_hash AS h FROM image_assets WHERE content_hash IS NOT NULL
          GROUP BY content_hash HAVING count(*)>1),
        m AS (SELECT c.h AS h, x.mark AS mark
          FROM c CROSS JOIN image_assets i ON i.content_hash=c.h CROSS JOIN asset_registry r ON r.id=i.id
          LEFT JOIN image_duplicate_marks x ON x.content_hash=i.content_hash AND x.asset_id=r.id
          WHERE {MEMBER_FILTER} {scope}),
        g AS (SELECT h, count(*) AS total, sum(mark IS NOT 'separated') AS n, sum(mark IS 'reviewed') AS seen
          FROM m GROUP BY h HAVING count(*)>1),
        s AS (SELECT g.h, g.n, (g.n<2 OR (coalesce(d.dismissed,0)=1 AND g.seen=g.n)) AS hidden
          FROM g LEFT JOIN image_duplicate_groups d ON d.content_hash=g.h)"
    )
}
/// Per-digest reads start from the digest index; the planner otherwise prefers
/// the registry's kind index and scans every image.
const BY_HASH: &str = "FROM image_assets i CROSS JOIN asset_registry r ON r.id=i.id";

fn read_group(db: &Connection, scope: Option<&[String]>, hash: &str) -> Result<Option<DuplicateGroup>> {
    let (filter, mut values) = scope_values(scope)?;
    let decision: Option<(u32, Option<String>, bool)> = db
        .query_row(
            "SELECT revision,preferred_id,dismissed FROM image_duplicate_groups WHERE content_hash=?1",
            [hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (revision, preferred, dismissed) = decision.unwrap_or((0, None, false));
    let mut args: Vec<Value> = vec![hash.to_owned().into()];
    args.append(&mut values);
    let counts: (u32, u32, u32) = db.query_row(
        &format!(
            "SELECT count(*), coalesce(sum(x.mark IS NOT 'separated'),0), coalesce(sum(x.mark IS 'reviewed'),0)
             {BY_HASH}
             LEFT JOIN image_duplicate_marks x ON x.content_hash=i.content_hash AND x.asset_id=r.id
             WHERE i.content_hash=? AND {MEMBER_FILTER} {filter}"
        ),
        rusqlite::params_from_iter(args.iter()),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let (total, member_count, seen) = counts;
    if total < 2 {
        return Ok(None);
    }
    let mut q = db.prepare_cached(&format!(
        "SELECT {SUMMARY_COLUMNS}, s.name, x.mark
         {BY_HASH} JOIN sources s ON s.id=r.source_id
         LEFT JOIN image_analyses p ON p.content_hash=i.content_hash AND p.analyzer_version=i.analyzer_version
         LEFT JOIN image_duplicate_marks x ON x.content_hash=i.content_hash AND x.asset_id=r.id
         WHERE i.content_hash=? AND {MEMBER_FILTER} {filter}
         ORDER BY x.mark IS 'separated', (CASE WHEN s.available=0 THEN 'offline' ELSE i.availability END)!='available',
          lower(r.relative_path), r.relative_path, r.id
         LIMIT {}",
        MAX_MEMBERS + 1
    ))?;
    let mut rows = q.query(rusqlite::params_from_iter(args.iter()))?;
    let mut members = vec![];
    let mut truncated = false;
    while let Some(row) = rows.next()? {
        if members.len() == MAX_MEMBERS {
            truncated = true;
            break;
        }
        let mark: Option<String> = row.get(12)?;
        members.push(DuplicateMember {
            image: summary(row)?,
            source_name: row.get(11)?,
            separated: mark.as_deref() == Some("separated"),
            new_since_dismissed: dismissed && mark.is_none(),
        });
    }
    let preferred_state = match &preferred {
        None => PreferredState::None,
        Some(id) => preferred_state(db, scope, hash, id)?,
    };
    // An out-of-scope preference is reported as no preference at all.
    let preferred = preferred.filter(|_| preferred_state != PreferredState::None);
    let active = |s: PreferredState| matches!(s, PreferredState::Available | PreferredState::Offline | PreferredState::Inaccessible);
    let representative_id = if active(preferred_state) {
        preferred.clone()
    } else {
        members.iter().find(|m| !m.separated).map(|m| m.image.header.id.clone())
    };
    Ok(Some(DuplicateGroup {
        kind: GroupKind::Exact,
        content_hash: hash.to_owned(),
        revision,
        member_count,
        separated_count: total - member_count,
        members,
        members_truncated: truncated,
        preferred_id: preferred,
        preferred_state,
        representative_id,
        dismissed,
        hidden: member_count < 2 || (dismissed && seen == member_count),
    }))
}

fn preferred_state(db: &Connection, scope: Option<&[String]>, hash: &str, id: &str) -> Result<PreferredState> {
    let row: Option<(String, String, Option<String>, String, String, bool, Option<String>)> = db
        .query_row(
            "SELECT r.kind, r.source_id, i.content_hash, i.technical_state,
              CASE WHEN s.available=0 THEN 'offline' ELSE i.availability END,
              s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path),
              (SELECT mark FROM image_duplicate_marks x WHERE x.content_hash=?2 AND x.asset_id=r.id)
             FROM asset_registry r JOIN image_assets i ON i.id=r.id JOIN sources s ON s.id=r.source_id WHERE r.id=?1",
            params![id, hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()?;
    let Some((kind, source, current, technical, availability, member, mark)) = row else {
        return Ok(PreferredState::None);
    };
    if kind != "image" || !member || scope.is_some_and(|ids| !ids.contains(&source)) {
        return Ok(PreferredState::None);
    }
    Ok(if current.as_deref() != Some(hash) || technical == "pending" {
        PreferredState::Changed
    } else if availability == "missing" {
        PreferredState::Missing
    } else if mark.as_deref() == Some("separated") {
        PreferredState::Separated
    } else if availability == "offline" {
        PreferredState::Offline
    } else if availability == "inaccessible" {
        PreferredState::Inaccessible
    } else {
        PreferredState::Available
    })
}

impl Catalog {
    /// Exact-duplicate stacks visible in `scope` (`None` = desktop owner). Copies
    /// outside the scope never contribute to counts, members or preferences, so a
    /// scoped caller cannot learn that identical bytes exist elsewhere.
    pub fn image_duplicate_groups(
        &self,
        scope: Option<&[String]>,
        include_hidden: bool,
        offset: u32,
        limit: u32,
    ) -> Result<DuplicatePage> {
        if limit == 0 || limit > MAX_GROUP_PAGE || offset > MAX_GROUP_OFFSET {
            return Err(E::InvalidArgument);
        }
        let (filter, values) = scope_values(scope)?;
        let tx = self.db.unchecked_transaction()?;
        let mut args = values.clone();
        args.push(Value::Integer(include_hidden.into()));
        args.push(i64::from(limit).into());
        args.push(i64::from(offset).into());
        // Largest open stacks first; hidden ones after, for review and undo. Totals
        // are window sums over every stack, so one evaluation serves both.
        let mut totals = None;
        let hashes = {
            let mut q = tx.prepare(&format!(
                "{}, t AS (SELECT h, n, hidden, sum(NOT hidden) OVER () AS open,
                  sum(CASE WHEN hidden THEN 0 ELSE n-1 END) OVER () AS extra, sum(hidden) OVER () AS hid FROM s)
                 SELECT h, open, extra, hid FROM t WHERE ? OR NOT hidden ORDER BY hidden, n DESC, h LIMIT ? OFFSET ?",
                stacks_cte(&filter)
            ))?;
            let mut rows = q.query(rusqlite::params_from_iter(args.iter()))?;
            let mut hashes = vec![];
            while let Some(row) = rows.next()? {
                totals = Some((row.get::<_, u32>(1)?, row.get::<_, u32>(2)?, row.get::<_, u32>(3)?));
                hashes.push(row.get::<_, String>(0)?);
            }
            hashes
        };
        let (open_groups, extra_copies, hidden_groups) = match totals {
            Some(t) => t,
            // Past the end: the counts still describe the whole view.
            None => tx.query_row(
                &format!(
                    "{} SELECT coalesce(sum(NOT hidden),0), coalesce(sum(CASE WHEN hidden THEN 0 ELSE n-1 END),0), coalesce(sum(hidden),0) FROM s",
                    stacks_cte(&filter)
                ),
                rusqlite::params_from_iter(values.iter()),
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?,
        };
        let mut groups = vec![];
        let mut bytes = 256;
        for h in hashes {
            let Some(group) = read_group(&tx, scope, &h)? else { continue };
            let size = serde_json::to_vec(&group)?.len() + 1;
            // A short page tells the caller where to resume; nothing is truncated silently.
            if bytes + size > MAX_BUNDLE_BYTES && !groups.is_empty() {
                break;
            }
            bytes += size;
            groups.push(group);
        }
        tx.commit()?;
        Ok(DuplicatePage {
            groups,
            offset,
            total: if include_hidden { open_groups + hidden_groups } else { open_groups },
            open_groups,
            extra_copies,
            hidden_groups,
        })
    }

    /// One stack, read again after a decision or a conflict.
    pub fn image_duplicate_group(&self, scope: Option<&[String]>, content_hash: &str) -> Result<DuplicateGroup> {
        if !valid_hash(content_hash) {
            return Err(E::InvalidArgument);
        }
        read_group(&self.db, scope, content_hash)?.ok_or(E::NotFound)
    }

    /// Applies one creator decision from the revision the caller last read.
    /// Only copies inside `scope` can be named or changed.
    pub fn update_image_duplicate_group(
        &mut self,
        scope: Option<&[String]>,
        content_hash: &str,
        expected_revision: u32,
        action: &DuplicateAction,
    ) -> Result<DuplicateGroup> {
        if !valid_hash(content_hash) {
            return Err(E::InvalidArgument);
        }
        let (filter, values) = scope_values(scope)?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(u32, Option<String>)> = tx
            .query_row(
                "SELECT revision,preferred_id FROM image_duplicate_groups WHERE content_hash=?1",
                [content_hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (revision, preferred) = current.unwrap_or((0, None));
        if revision != expected_revision {
            return Err(E::Conflict);
        }
        let mut args: Vec<Value> = vec![content_hash.to_owned().into()];
        args.extend(values.iter().cloned());
        let members: HashMap<String, bool> = {
            let mut q = tx.prepare(&format!(
                "SELECT r.id, x.mark IS 'separated' {BY_HASH}
                 LEFT JOIN image_duplicate_marks x ON x.content_hash=i.content_hash AND x.asset_id=r.id
                 WHERE i.content_hash=? AND {MEMBER_FILTER} {filter}"
            ))?;
            let rows = q.query_map(rusqlite::params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<std::result::Result<_, _>>()?
        };
        if members.len() < 2 {
            return Err(E::NotFound);
        }
        let member = |id: &str| -> Result<bool> {
            if id.len() != 36 {
                return Err(E::InvalidArgument);
            }
            members.get(id).copied().ok_or(E::NotFound)
        };
        let next = revision.checked_add(1).ok_or(E::Conflict)?;
        let now = crate::jobs::now_secs();
        // The group row exists before any mark references it.
        tx.execute(
            "INSERT INTO image_duplicate_groups(content_hash,revision,updated_at) VALUES(?1,?2,?3)
             ON CONFLICT(content_hash) DO UPDATE SET revision=excluded.revision,updated_at=excluded.updated_at",
            params![content_hash, next, now],
        )?;
        let separate_mark = |tx: &Connection, id: &str, mark: Option<&str>| -> Result<()> {
            match mark {
                Some(m) => tx.execute(
                    "INSERT INTO image_duplicate_marks(content_hash,asset_id,mark) VALUES(?1,?2,?3)
                     ON CONFLICT(content_hash,asset_id) DO UPDATE SET mark=excluded.mark",
                    params![content_hash, id, m],
                )?,
                None => tx.execute(
                    "DELETE FROM image_duplicate_marks WHERE content_hash=?1 AND asset_id=?2",
                    params![content_hash, id],
                )?,
            };
            Ok(())
        };
        // A scoped caller never overrides a preference it cannot see.
        let foreign_preferred = scope.is_some() && preferred.as_deref().is_some_and(|p| !members.contains_key(p));
        match action {
            DuplicateAction::Prefer { asset_id } => {
                if member(asset_id)? {
                    return Err(E::InvalidArgument);
                }
                if foreign_preferred {
                    return Err(E::PermissionDenied);
                }
                tx.execute(
                    "UPDATE image_duplicate_groups SET preferred_id=?2 WHERE content_hash=?1",
                    params![content_hash, asset_id],
                )?;
            }
            DuplicateAction::ClearPreferred => {
                if foreign_preferred {
                    return Err(E::PermissionDenied);
                }
                tx.execute("UPDATE image_duplicate_groups SET preferred_id=NULL WHERE content_hash=?1", [content_hash])?;
            }
            DuplicateAction::Separate { asset_id } => {
                if member(asset_id)? {
                    return Err(E::InvalidArgument);
                }
                separate_mark(&tx, asset_id, Some("separated"))?;
                tx.execute(
                    "UPDATE image_duplicate_groups SET preferred_id=NULL WHERE content_hash=?1 AND preferred_id=?2",
                    params![content_hash, asset_id],
                )?;
            }
            DuplicateAction::Rejoin { asset_id } => {
                if !member(asset_id)? {
                    return Err(E::InvalidArgument);
                }
                // Rejoining is itself a review: it does not resurface a dismissed stack.
                let dismissed: bool = tx.query_row(
                    "SELECT dismissed FROM image_duplicate_groups WHERE content_hash=?1",
                    [content_hash],
                    |r| r.get(0),
                )?;
                separate_mark(&tx, asset_id, dismissed.then_some("reviewed"))?;
            }
            DuplicateAction::Dismiss => {
                let mut scope_args: Vec<Value> = vec![content_hash.to_owned().into()];
                scope_args.extend(values.iter().cloned());
                tx.execute(
                    "DELETE FROM image_duplicate_marks WHERE content_hash=?1 AND mark='reviewed'",
                    [content_hash],
                )?;
                tx.execute(
                    &format!(
                        "INSERT INTO image_duplicate_marks(content_hash,asset_id,mark)
                         SELECT i.content_hash, r.id, 'reviewed' {BY_HASH}
                         WHERE i.content_hash=? AND {MEMBER_FILTER} {filter}
                         AND NOT EXISTS(SELECT 1 FROM image_duplicate_marks x WHERE x.content_hash=i.content_hash AND x.asset_id=r.id)"
                    ),
                    rusqlite::params_from_iter(scope_args.iter()),
                )?;
                tx.execute("UPDATE image_duplicate_groups SET dismissed=1 WHERE content_hash=?1", [content_hash])?;
            }
            DuplicateAction::Restore => {
                tx.execute(
                    "DELETE FROM image_duplicate_marks WHERE content_hash=?1 AND mark='reviewed'",
                    [content_hash],
                )?;
                tx.execute("UPDATE image_duplicate_groups SET dismissed=0 WHERE content_hash=?1", [content_hash])?;
            }
        }
        // Marks for copies whose bytes have since changed no longer describe this stack.
        tx.execute(
            "DELETE FROM image_duplicate_marks WHERE content_hash=?1
             AND asset_id NOT IN (SELECT id FROM image_assets WHERE content_hash=?1)",
            [content_hash],
        )?;
        let group = read_group(&tx, scope, content_hash)?.ok_or(E::NotFound)?;
        tx.commit()?;
        Ok(group)
    }
}

/// Decisions as they travel in portable catalogs and folder snapshots. Every
/// referenced copy must be an image record of the same document holding these
/// exact bytes, so a folder snapshot never names a file outside its folder.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PortableDuplicateDecision {
    pub content_hash: String,
    pub preferred_id: Option<String>,
    pub dismissed: bool,
    pub separated: Vec<String>,
    pub reviewed: Vec<String>,
}

/// Decisions restricted to the exported images. Unreferenced or stale parts drop
/// out: a preference for a copy whose bytes changed is not a decision about it.
pub(crate) fn export_decisions(db: &Connection, images: &[PortableImage]) -> crate::Result<Vec<PortableDuplicateDecision>> {
    let current: HashMap<&str, &str> = images
        .iter()
        .filter(|i| i.asset.header.kind == crate::image_contracts::AssetKind::Image)
        .filter_map(|i| Some((i.asset.header.id.as_str(), i.asset.header.content_hash.as_deref()?)))
        .collect();
    let mut groups = db.prepare("SELECT content_hash,preferred_id,dismissed FROM image_duplicate_groups ORDER BY content_hash")?;
    let mut marks = db.prepare_cached("SELECT asset_id,mark FROM image_duplicate_marks WHERE content_hash=?1 ORDER BY asset_id")?;
    let rows = groups
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, bool>(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut out = vec![];
    for (hash, preferred, dismissed) in rows {
        let holds = |id: &str| current.get(id) == Some(&hash.as_str());
        let mut separated = vec![];
        let mut reviewed = vec![];
        for mark in marks.query_map([&hash], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, mark) = mark?;
            if holds(&id) {
                if mark == "separated" { separated.push(id) } else { reviewed.push(id) }
            }
        }
        let preferred_id = preferred.filter(|p| holds(p) && !separated.contains(p));
        if preferred_id.is_some() || !separated.is_empty() || !reviewed.is_empty() {
            out.push(PortableDuplicateDecision { content_hash: hash, preferred_id, dismissed: dismissed && !reviewed.is_empty(), separated, reviewed });
        }
    }
    Ok(out)
}

pub(crate) fn validate_decisions(decisions: &[PortableDuplicateDecision], images: &[PortableImage]) -> crate::Result<()> {
    let bad = || crate::invalid("Invalid duplicate decision");
    if decisions.len() > crate::mixed_catalog::MAX_IMAGES {
        return Err(bad());
    }
    let current: HashMap<&str, &str> = images
        .iter()
        .filter(|i| i.asset.header.kind == crate::image_contracts::AssetKind::Image)
        .filter_map(|i| Some((i.asset.header.id.as_str(), i.asset.header.content_hash.as_deref()?)))
        .collect();
    let mut hashes = HashSet::new();
    let mut named = HashSet::new();
    for d in decisions {
        if !valid_hash(&d.content_hash) || !hashes.insert(d.content_hash.as_str()) {
            return Err(bad());
        }
        let refs = d.separated.iter().chain(&d.reviewed);
        let mut local = HashSet::new();
        for id in refs.clone() {
            // One copy holds one digest, so it appears in at most one decision.
            if current.get(id.as_str()) != Some(&d.content_hash.as_str()) || !local.insert(id) || !named.insert(id) {
                return Err(bad());
            }
        }
        if d.preferred_id.as_ref().is_some_and(|p| {
            current.get(p.as_str()) != Some(&d.content_hash.as_str()) || d.separated.contains(p)
        }) || (d.dismissed && d.reviewed.is_empty())
            || (d.preferred_id.is_none() && local.is_empty())
        {
            return Err(bad());
        }
    }
    Ok(())
}

/// Inside the caller's import transaction. A stack this library already decided
/// keeps its own preference and dismissal; imported copies add their marks.
pub(crate) fn import_decisions(tx: &Connection, decisions: &[PortableDuplicateDecision]) -> crate::Result<()> {
    let now = crate::jobs::now_secs();
    for d in decisions {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM image_duplicate_groups WHERE content_hash=?1)",
            [&d.content_hash],
            |r| r.get(0),
        )?;
        if exists {
            tx.execute(
                "UPDATE image_duplicate_groups SET revision=revision+1,updated_at=?2,
                 preferred_id=coalesce(preferred_id,?3) WHERE content_hash=?1 AND revision<4294967295",
                params![d.content_hash, now, d.preferred_id],
            )?;
        } else {
            tx.execute(
                "INSERT INTO image_duplicate_groups(content_hash,revision,preferred_id,dismissed,updated_at) VALUES(?1,1,?2,?3,?4)",
                params![d.content_hash, d.preferred_id, d.dismissed, now],
            )?;
        }
        for (ids, mark) in [(&d.separated, "separated"), (&d.reviewed, "reviewed")] {
            for id in ids {
                tx.execute(
                    "INSERT OR IGNORE INTO image_duplicate_marks(content_hash,asset_id,mark) VALUES(?1,?2,?3)",
                    params![d.content_hash, id, mark],
                )?;
            }
        }
    }
    Ok(())
}

/// Restore check for a copied database file, where triggers did not run.
pub(crate) fn validate_duplicate_tables(db: &Connection) -> crate::Result<()> {
    let wrong: i64 = db.query_row(
        "SELECT (SELECT count(*) FROM image_duplicate_groups g JOIN asset_registry r ON r.id=g.preferred_id WHERE r.kind!='image')
         + (SELECT count(*) FROM image_duplicate_marks x JOIN asset_registry r ON r.id=x.asset_id WHERE r.kind!='image')",
        [],
        |r| r.get(0),
    )?;
    if wrong != 0 {
        return Err(crate::invalid("Backup duplicate stacks reference non-image assets"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v15_database(path: &std::path::Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        for schema in [
            include_str!("schema.sql"),
            include_str!("source_catalog_schema.sql"),
            include_str!("settings_schema.sql"),
            include_str!("search_profile_schema.sql"),
            include_str!("image_schema.sql"),
            include_str!("image_jobs_schema.sql"),
            include_str!("gallery_schema.sql"),
            include_str!("annotation_schema.sql"),
            include_str!("image_refresh_schema.sql"),
            include_str!("mixed_catalog_schema.sql"),
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 15).unwrap();
        db
    }
    fn master(db: &Connection) -> Vec<(String, String)> {
        let mut q = db.prepare("SELECT name,replace(replace(coalesce(sql,''),' ',''),char(10),'') FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").unwrap();
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        rows.collect::<std::result::Result<_, _>>().unwrap()
    }
    fn ready(c: &mut Catalog, source: &crate::catalog::Source, path: &str, hash: &str) -> String {
        let id = c.register_image(source, path, Some(hash)).unwrap().header.id;
        c.db.execute("UPDATE image_assets SET technical_state='failed' WHERE id=?1", [&id]).unwrap();
        id
    }

    #[test]
    fn genuine_v15_upgrade_adds_empty_decision_tables_with_a_backup() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v15.sqlite");
        let mut old = Catalog { db: v15_database(&path) };
        let source = old.add_source(d.path()).unwrap();
        let hash = "c".repeat(64);
        let a = ready(&mut old, &source, "a.png", &hash);
        let b = ready(&mut old, &source, "b.png", &hash);
        let before = (old.image(&a, None).unwrap(), old.image(&b, None).unwrap());
        drop(old);
        let mut c = Catalog::open(&path).unwrap();
        assert_eq!(c.db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), crate::catalog::SCHEMA_VERSION);
        assert_eq!((c.image(&a, None).unwrap(), c.image(&b, None).unwrap()), before);
        // Existing copies form a stack immediately; no recomputation pass is needed.
        let page = c.image_duplicate_groups(None, false, 0, 10).unwrap();
        assert_eq!((page.open_groups, page.groups[0].revision), (1, 0));
        let fresh = Catalog::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(master(&c.db), master(&fresh.db), "restore compares schemas exactly");
        let backup = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(&format!("pre-v{}-", crate::catalog::SCHEMA_VERSION))).unwrap();
        let db = Connection::open(backup).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 15);
        // Decisions mark the folder dirty for its next snapshot.
        c.db.execute("UPDATE source_catalog_state SET dirty=0", []).unwrap();
        c.update_image_duplicate_group(None, &hash, 0, &DuplicateAction::Prefer { asset_id: b.clone() }).unwrap();
        let dirty: i64 = c.db.query_row("SELECT dirty FROM source_catalog_state WHERE source_id=?1", [&source.id], |r| r.get(0)).unwrap();
        assert!(dirty > 0);
        // Deleting the preferred copy clears the preference; it never deletes others.
        c.delete_image(&source, &b, Some(&hash), 1).unwrap();
        let preferred: Option<String> = c.db.query_row("SELECT preferred_id FROM image_duplicate_groups", [], |r| r.get(0)).unwrap();
        assert_eq!(preferred, None);
        assert!(c.image(&a, None).is_ok());
    }

    #[test]
    fn v16_collision_rolls_back_and_keeps_the_v15_catalog_usable() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v15.sqlite");
        let db = v15_database(&path);
        db.execute_batch("CREATE TABLE image_duplicate_marks(kept TEXT); INSERT INTO image_duplicate_marks VALUES('retained');").unwrap();
        drop(db);
        assert!(Catalog::open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 15);
        assert_eq!(db.query_row("SELECT kept FROM image_duplicate_marks", [], |r| r.get::<_, String>(0)).unwrap(), "retained");
        let groups: i64 = db.query_row("SELECT count(*) FROM sqlite_master WHERE name='image_duplicate_groups'", [], |r| r.get(0)).unwrap();
        assert_eq!(groups, 0, "the partial v16 script was rolled back");
    }

    #[test]
    fn audio_and_elements_never_join_stacks_and_triggers_refuse_them() {
        let d = tempfile::tempdir().unwrap();
        let mut c = Catalog::open(&d.path().join("db.sqlite")).unwrap();
        let source = c.add_source(d.path()).unwrap();
        let hash = "d".repeat(64);
        let a = ready(&mut c, &source, "a.png", &hash);
        let _pending = c.register_image(&source, "pending.png", Some(&hash)).unwrap();
        // A pending copy awaits re-verification, so one verified copy is not a stack.
        assert_eq!(c.image_duplicate_groups(None, true, 0, 10).unwrap().total, 0);
        let sound = c.register(&source, "tone.wav", &hash).unwrap();
        assert!(c.db.execute("INSERT INTO image_duplicate_groups(content_hash,revision,preferred_id,updated_at) VALUES(?1,1,?2,0)", params![hash, sound]).is_err());
        c.db.execute("INSERT INTO image_duplicate_groups(content_hash,revision,updated_at) VALUES(?1,1,0)", [&hash]).unwrap();
        assert!(c.db.execute("INSERT INTO image_duplicate_marks VALUES(?1,?2,'separated')", params![hash, sound]).is_err());
        assert!(c.db.execute("UPDATE image_duplicate_groups SET preferred_id=?1", [&sound]).is_err());
        c.db.execute("UPDATE image_duplicate_groups SET preferred_id=?1", [&a]).unwrap();
        crate::image_catalog::validate_image_database(&c.db).unwrap();
        for bad in ["", "x", &"G".repeat(64)] {
            assert_eq!(c.image_duplicate_group(None, bad).unwrap_err(), E::InvalidArgument);
        }
        assert_eq!(c.image_duplicate_groups(None, false, 0, 0).unwrap_err(), E::InvalidArgument);
        assert_eq!(c.image_duplicate_groups(None, false, 0, MAX_GROUP_PAGE + 1).unwrap_err(), E::InvalidArgument);
        assert_eq!(c.image_duplicate_groups(None, false, MAX_GROUP_OFFSET + 1, 1).unwrap_err(), E::InvalidArgument);
        let too_many: Vec<String> = (0..129).map(|i| i.to_string()).collect();
        assert_eq!(c.image_duplicate_groups(Some(&too_many), false, 0, 1).unwrap_err(), E::InvalidArgument);
    }
}
