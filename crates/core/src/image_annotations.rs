//! SS-046 creator-owned image annotations: explicit tag/note/favorite/rating
//! operations behind optimistic revisions, bounded reversible bulk actions,
//! library-wide tag rename/aliases, and their durable undo history.
//!
//! User facts live only in `image_assets.annotations` (the SS-040 contract);
//! technical facts stay in `image_analyses` and future AI suggestions in
//! `evidence`, so reanalysis or content replacement never rewrites them.
//! Writes go through `UPDATE image_assets`, whose shared trigger marks the
//! source snapshot dirty. Notes and tags are untrusted plain text.
use crate::{
    catalog::Catalog,
    image_catalog::{load, validate},
    image_contracts::{Annotations, MAX_RECORDS, MAX_TAGS, MAX_TAG_BYTES, MAX_TEXT_BYTES},
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

/// Display limit shared with audio tags (characters); the contract also caps bytes.
pub const MAX_TAG_CHARS: usize = 64;
/// Items in one bulk edit (the contract's record bound).
pub const MAX_BATCH: usize = MAX_RECORDS;
/// Images one rename may change; keeps a single history record bounded (~1 MiB).
pub const MAX_RENAME: usize = 10_000;
/// Retained undoable actions; older ones are pruned.
pub const MAX_HISTORY: i64 = 100;
pub const MAX_ALIASES: i64 = 1_000;
/// Distinct tags returned by a tag listing.
pub const MAX_TAG_LIST: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationError {
    #[error("Invalid annotation request")]
    InvalidArgument,
    #[error("Image not found")]
    NotFound,
    #[error("Image is outside its approved scope")]
    PermissionDenied,
    #[error("Annotations changed elsewhere; refresh and retry")]
    Conflict,
    #[error("Source is offline or the original is unavailable")]
    SourceUnavailable,
    #[error("Annotation exceeds a limit")]
    LimitExceeded,
    #[error("Annotation operation failed")]
    Internal,
}
impl From<rusqlite::Error> for AnnotationError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Internal
    }
}
impl From<serde_json::Error> for AnnotationError {
    fn from(_: serde_json::Error) -> Self {
        Self::Internal
    }
}
type Result<T> = std::result::Result<T, AnnotationError>;
use AnnotationError as E;

/// Display spelling, consistent with audio tags (underscores as spaces, collapsed
/// whitespace, ≤ 64 characters, no control characters) plus Unicode NFC.
/// `None` means the input was blank.
pub fn display_tag(raw: &str) -> Result<Option<String>> {
    let spaced = raw.nfc().collect::<String>().replace('_', " ");
    let display = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if display.is_empty() {
        return Ok(None);
    }
    if display.chars().any(char::is_control) {
        return Err(E::InvalidArgument);
    }
    if display.chars().count() > MAX_TAG_CHARS || display.len() > MAX_TAG_BYTES {
        return Err(E::LimitExceeded);
    }
    Ok(Some(display))
}
/// Case-insensitive identity of a display tag; the first spelling is kept.
pub fn tag_key(display: &str) -> String {
    display.nfc().collect::<String>().to_lowercase().nfc().collect()
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Explicit changes. Absent fields are kept; `rating: null` clears the rating.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationPatch {
    #[serde(default)]
    pub add_tags: Vec<String>,
    #[serde(default)]
    pub remove_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub rating: Option<Option<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemOutcome {
    Applied,
    Unchanged,
    Conflict,
    NotFound,
    PermissionDenied,
    SourceUnavailable,
    LimitExceeded,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemResult {
    pub id: String,
    pub outcome: ItemOutcome,
    /// Current revision after the operation, when the image is readable.
    pub revision: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditResult {
    pub annotations: Annotations,
    /// History entry for undo; `None` when nothing changed.
    pub action_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BulkTarget {
    pub id: String,
    pub revision: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagEffect {
    pub tag: String,
    /// Eligible images that already have (add) or currently carry (remove) this tag.
    pub count: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BulkPreview {
    /// Revision tokens to apply with; images changed after preview become conflicts.
    pub targets: Vec<BulkTarget>,
    pub changes: u32,
    pub unchanged: u32,
    /// Images that cannot be edited now (missing, out of scope or offline).
    pub skipped: Vec<ItemResult>,
    pub additions: Vec<TagEffect>,
    pub removals: Vec<TagEffect>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BulkResult {
    pub action_id: Option<String>,
    pub results: Vec<ItemResult>,
    pub applied: u32,
    pub unchanged: u32,
    pub conflicts: u32,
    pub skipped: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Edit,
    Bulk,
    Rename,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    Applied,
    Undone,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSummary {
    pub id: String,
    pub kind: ActionKind,
    pub label: String,
    pub state: ActionState,
    pub created_at: i64,
    pub items: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagAlias {
    pub alias: String,
    pub canonical: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagList {
    pub tags: Vec<TagCount>,
    pub truncated: bool,
    pub aliases: Vec<TagAlias>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenamePreview {
    pub from: String,
    pub to: String,
    /// Editable images in scope carrying `from`.
    pub images: u32,
    /// Of those, images that already carry `to` (the two tags merge).
    pub merged: u32,
    /// Images carrying `from` that cannot be edited now and keep it.
    pub unavailable: u32,
    /// Audio sounds with the same tag. Rename never changes audio tags.
    pub audio_unchanged: u32,
}

/// Inverse of one item's change, valid while the image is still at `after`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Entry {
    id: String,
    after: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    added: Vec<String>,
    /// Removed tags with their original positions, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    removed: Vec<(usize, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    favorite: Option<bool>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    rating: Option<Option<u8>>,
}
/// Alias state a rename created or re-pointed, restored on undo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct AliasChange {
    created: Option<(String, TagAlias)>,
    /// (alias_key, previous canonical) of aliases re-pointed to the new name.
    repointed: Vec<(String, String)>,
}

struct Prepared {
    add: Vec<String>,
    remove: HashSet<String>,
    comment: Option<String>,
    favorite: Option<bool>,
    rating: Option<Option<u8>>,
}

fn aliases(db: &Connection) -> Result<HashMap<String, String>> {
    let mut s = db.prepare("SELECT alias_key,canonical FROM image_tag_aliases")?;
    let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}
fn prepare(db: &Connection, p: &AnnotationPatch) -> Result<Prepared> {
    if p.add_tags.len() > MAX_TAGS || p.remove_tags.len() > MAX_TAGS {
        return Err(E::LimitExceeded);
    }
    if let Some(c) = &p.comment {
        if c.len() > MAX_TEXT_BYTES {
            return Err(E::LimitExceeded);
        }
        if c.contains('\0') {
            return Err(E::InvalidArgument);
        }
    }
    // The documented scale is 1–5 stars; "no rating" is null, never zero.
    if matches!(p.rating, Some(Some(r)) if !(1..=5).contains(&r)) {
        return Err(E::InvalidArgument);
    }
    let alias = aliases(db)?;
    let resolve = |t: String| alias.get(&tag_key(&t)).cloned().unwrap_or(t);
    let mut add: Vec<String> = vec![];
    for raw in &p.add_tags {
        if let Some(t) = display_tag(raw)?.map(resolve) {
            if !add.iter().any(|a| tag_key(a) == tag_key(&t)) {
                add.push(t);
            }
        }
    }
    let mut remove = HashSet::new();
    for raw in &p.remove_tags {
        if let Some(t) = display_tag(raw)? {
            remove.insert(tag_key(&t));
            remove.insert(tag_key(&resolve(t)));
        }
    }
    if add.iter().any(|t| remove.contains(&tag_key(t))) {
        return Err(E::InvalidArgument);
    }
    Ok(Prepared {
        add,
        remove,
        comment: p.comment.clone(),
        favorite: p.favorite,
        rating: p.rating,
    })
}
/// The changed annotations and their inverse, or `None` when nothing changes.
fn change(a: &Annotations, p: &Prepared) -> Result<Option<(Annotations, Entry)>> {
    let mut next = a.clone();
    let mut entry = Entry { id: String::new(), after: 0, added: vec![], removed: vec![], comment: None, favorite: None, rating: None };
    for (i, t) in a.tags.iter().enumerate() {
        if p.remove.contains(&tag_key(t)) {
            entry.removed.push((i, t.clone()));
        }
    }
    next.tags.retain(|t| !p.remove.contains(&tag_key(t)));
    for t in &p.add {
        if !next.tags.iter().any(|x| tag_key(x) == tag_key(t)) {
            next.tags.push(t.clone());
            entry.added.push(t.clone());
        }
    }
    if next.tags.len() > MAX_TAGS {
        return Err(E::LimitExceeded);
    }
    if let Some(c) = &p.comment {
        if *c != a.comment {
            entry.comment = Some(a.comment.clone());
            next.comment = c.clone();
        }
    }
    if let Some(f) = p.favorite {
        if f != a.favorite {
            entry.favorite = Some(a.favorite);
            next.favorite = f;
        }
    }
    if let Some(r) = p.rating {
        if r != a.rating {
            entry.rating = Some(a.rating);
            next.rating = r;
        }
    }
    let changed = !entry.added.is_empty() || !entry.removed.is_empty() || entry.comment.is_some() || entry.favorite.is_some() || entry.rating.is_some();
    Ok(changed.then_some((next, entry)))
}
fn revert(a: &Annotations, e: &Entry) -> Annotations {
    let mut next = a.clone();
    let added: HashSet<_> = e.added.iter().map(|t| tag_key(t)).collect();
    next.tags.retain(|t| !added.contains(&tag_key(t)));
    for (i, t) in &e.removed {
        if !next.tags.iter().any(|x| tag_key(x) == tag_key(t)) {
            let at = (*i).min(next.tags.len());
            next.tags.insert(at, t.clone());
        }
    }
    if let Some(c) = &e.comment {
        next.comment = c.clone();
    }
    if let Some(f) = e.favorite {
        next.favorite = f;
    }
    if let Some(r) = e.rating {
        next.rating = r;
    }
    next
}

/// Current asset identity, scope and availability; the error is the item outcome.
fn editable(db: &Connection, id: &str) -> std::result::Result<Annotations, ItemOutcome> {
    let row: Option<(String, bool, bool, String)> = db
        .query_row(
            "SELECT r.kind,s.available,(s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f
              WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path)),i.availability
             FROM asset_registry r JOIN sources s ON s.id=r.source_id JOIN image_assets i ON i.id=r.id WHERE r.id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| ItemOutcome::NotFound)?;
    let Some((kind, online, member, availability)) = row else { return Err(ItemOutcome::NotFound) };
    if kind != "image" {
        return Err(ItemOutcome::NotFound);
    }
    if !member {
        return Err(ItemOutcome::PermissionDenied);
    }
    // Matches audio: annotations are not edited while the original is offline or missing.
    if !online || availability != "available" {
        return Err(ItemOutcome::SourceUnavailable);
    }
    load(db, id).map(|a| a.annotations).map_err(|_| ItemOutcome::NotFound)
}
fn write(tx: &Transaction, id: &str, current: &Annotations, mut next: Annotations) -> Result<Annotations> {
    next.revision = current.revision.checked_add(1).ok_or(E::LimitExceeded)?;
    let mut asset = load(tx, id).map_err(|_| E::NotFound)?;
    asset.annotations = next.clone();
    validate(&asset).map_err(|_| E::LimitExceeded)?;
    let updated = tx.execute(
        "UPDATE image_assets SET annotation_revision=?2,annotations=?3 WHERE id=?1 AND annotation_revision=?4",
        params![id, next.revision, serde_json::to_string(&next)?, current.revision],
    )?;
    if updated != 1 {
        return Err(E::Conflict);
    }
    Ok(next)
}
fn record(tx: &Transaction, kind: ActionKind, label: &str, entries: &[Entry], alias: Option<&AliasChange>) -> Result<Option<String>> {
    if entries.is_empty() && alias.is_none_or(|a| a.created.is_none() && a.repointed.is_empty()) {
        return Ok(None);
    }
    let id = Uuid::new_v4().to_string();
    let kind_name = serde_json::to_value(kind)?;
    tx.execute(
        "INSERT INTO image_annotation_actions(id,kind,label,state,created_at,items,entries,alias) VALUES(?1,?2,?3,'applied',?4,?5,?6,?7)",
        params![
            id,
            kind_name.as_str(),
            label,
            crate::jobs::now_secs(),
            entries.len().max(1) as i64,
            serde_json::to_string(entries)?,
            alias.map(serde_json::to_string).transpose()?
        ],
    )?;
    tx.execute(
        "DELETE FROM image_annotation_actions WHERE seq <= (SELECT max(seq) FROM image_annotation_actions) - ?1",
        [MAX_HISTORY],
    )?;
    Ok(Some(id))
}
fn quoted(tags: &[String]) -> String {
    let mut s = tags.iter().take(3).map(|t| format!("“{t}”")).collect::<Vec<_>>().join(", ");
    if tags.len() > 3 {
        s.push_str(&format!(" +{}", tags.len() - 3));
    }
    s
}
fn label(p: &Prepared, removed: &[String], items: usize) -> String {
    let mut parts = vec![];
    if !p.add.is_empty() {
        parts.push(format!("Add {}", quoted(&p.add)));
    }
    if !removed.is_empty() {
        parts.push(format!("remove {}", quoted(removed)));
    }
    if p.comment.is_some() {
        parts.push("edit notes".into());
    }
    match p.favorite {
        Some(true) => parts.push("mark favorite".into()),
        Some(false) => parts.push("unmark favorite".into()),
        None => {}
    }
    match p.rating {
        Some(Some(r)) => parts.push(format!("rate {r}/5")),
        Some(None) => parts.push("clear rating".into()),
        None => {}
    }
    let mut text = parts.join(", ");
    if let Some(first) = text.get(0..1) {
        text = first.to_uppercase() + &text[1..];
    }
    let text = format!("{text} · {items} image{}", if items == 1 { "" } else { "s" });
    truncate(text)
}
fn truncate(mut s: String) -> String {
    if s.len() > 1024 {
        let mut end = 1020;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}
fn tally(results: &[ItemResult], action_id: Option<String>) -> BulkResult {
    let count = |o: &[ItemOutcome]| results.iter().filter(|r| o.contains(&r.outcome)).count() as u32;
    BulkResult {
        action_id,
        applied: count(&[ItemOutcome::Applied]),
        unchanged: count(&[ItemOutcome::Unchanged]),
        conflicts: count(&[ItemOutcome::Conflict]),
        skipped: count(&[ItemOutcome::NotFound, ItemOutcome::PermissionDenied, ItemOutcome::SourceUnavailable, ItemOutcome::LimitExceeded]),
        results: results.to_vec(),
    }
}
fn unique(ids: impl IntoIterator<Item = String>) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let ids: Vec<String> = ids.into_iter().filter(|id| seen.insert(id.clone())).collect();
    if ids.is_empty() {
        return Err(E::InvalidArgument);
    }
    if ids.len() > MAX_BATCH {
        return Err(E::LimitExceeded);
    }
    Ok(ids)
}
fn item_error(o: ItemOutcome) -> AnnotationError {
    match o {
        ItemOutcome::PermissionDenied => E::PermissionDenied,
        ItemOutcome::SourceUnavailable => E::SourceUnavailable,
        ItemOutcome::LimitExceeded => E::LimitExceeded,
        ItemOutcome::Conflict => E::Conflict,
        _ => E::NotFound,
    }
}

impl Catalog {
    fn immediate(&mut self) -> Result<Transaction<'_>> {
        Ok(self.db.transaction_with_behavior(TransactionBehavior::Immediate)?)
    }

    /// One image, from the revision the editor last read. A concurrent change is
    /// a `Conflict`, never silently merged into the editor's view.
    pub fn edit_image_annotations(&mut self, id: &str, expected_revision: u32, patch: &AnnotationPatch) -> Result<EditResult> {
        let tx = self.immediate()?;
        let p = prepare(&tx, patch)?;
        let current = editable(&tx, id).map_err(item_error)?;
        if current.revision != expected_revision {
            return Err(E::Conflict);
        }
        let Some((next, mut entry)) = change(&current, &p)? else {
            return Ok(EditResult { annotations: current, action_id: None });
        };
        let removed: Vec<String> = entry.removed.iter().map(|(_, t)| t.clone()).collect();
        let next = write(&tx, id, &current, next)?;
        entry.id = id.into();
        entry.after = next.revision;
        let action_id = record(&tx, ActionKind::Edit, &label(&p, &removed, 1), &[entry], None)?;
        tx.commit()?;
        Ok(EditResult { annotations: next, action_id })
    }

    /// Count and revision tokens for a bulk edit, without writing anything.
    pub fn preview_bulk_annotations(&self, ids: &[String], patch: &AnnotationPatch) -> Result<BulkPreview> {
        if patch.comment.is_some() {
            return Err(E::InvalidArgument);
        }
        let ids = unique(ids.iter().cloned())?;
        let p = prepare(&self.db, patch)?;
        let removal_tags: Vec<String> = patch.remove_tags.iter().filter_map(|t| display_tag(t).ok().flatten()).collect();
        let (mut targets, mut skipped) = (vec![], vec![]);
        let (mut changes, mut unchanged) = (0, 0);
        let mut additions: Vec<TagEffect> = p.add.iter().map(|t| TagEffect { tag: t.clone(), count: 0 }).collect();
        let mut removals: Vec<TagEffect> = removal_tags.iter().map(|t| TagEffect { tag: t.clone(), count: 0 }).collect();
        for id in ids {
            match editable(&self.db, &id) {
                Err(outcome) => skipped.push(ItemResult { id, outcome, revision: None }),
                Ok(a) => {
                    let keys: HashSet<String> = a.tags.iter().map(|t| tag_key(t)).collect();
                    for e in additions.iter_mut().chain(removals.iter_mut()) {
                        e.count += u32::from(keys.contains(&tag_key(&e.tag)));
                    }
                    match change(&a, &p) {
                        Ok(Some(_)) => changes += 1,
                        Ok(None) => unchanged += 1,
                        Err(_) => {
                            skipped.push(ItemResult { id, outcome: ItemOutcome::LimitExceeded, revision: Some(a.revision) });
                            continue;
                        }
                    }
                    targets.push(BulkTarget { id, revision: a.revision });
                }
            }
        }
        Ok(BulkPreview { targets, changes, unchanged, skipped, additions, removals })
    }

    /// Applies one bulk edit in a single transaction with per-item results.
    /// Items changed since preview are `Conflict` and left untouched; a storage
    /// failure rolls back every item and the history record together.
    pub fn apply_bulk_annotations(&mut self, targets: &[BulkTarget], patch: &AnnotationPatch) -> Result<BulkResult> {
        if patch.comment.is_some() {
            return Err(E::InvalidArgument);
        }
        let ids = unique(targets.iter().map(|t| t.id.clone()))?;
        if ids.len() != targets.len() {
            return Err(E::InvalidArgument);
        }
        let tx = self.immediate()?;
        let p = prepare(&tx, patch)?;
        let (mut results, mut entries, mut removed) = (vec![], vec![], vec![]);
        for t in targets {
            let current = match editable(&tx, &t.id) {
                Err(outcome) => {
                    results.push(ItemResult { id: t.id.clone(), outcome, revision: None });
                    continue;
                }
                Ok(a) => a,
            };
            if current.revision != t.revision {
                results.push(ItemResult { id: t.id.clone(), outcome: ItemOutcome::Conflict, revision: Some(current.revision) });
                continue;
            }
            match change(&current, &p) {
                Ok(None) => results.push(ItemResult { id: t.id.clone(), outcome: ItemOutcome::Unchanged, revision: Some(current.revision) }),
                Err(_) => results.push(ItemResult { id: t.id.clone(), outcome: ItemOutcome::LimitExceeded, revision: Some(current.revision) }),
                Ok(Some((next, mut entry))) => {
                    for (_, tag) in &entry.removed {
                        if !removed.iter().any(|r: &String| tag_key(r) == tag_key(tag)) {
                            removed.push(tag.clone());
                        }
                    }
                    let next = write(&tx, &t.id, &current, next)?;
                    entry.id = t.id.clone();
                    entry.after = next.revision;
                    entries.push(entry);
                    results.push(ItemResult { id: t.id.clone(), outcome: ItemOutcome::Applied, revision: Some(next.revision) });
                }
            }
        }
        let action_id = record(&tx, ActionKind::Bulk, &label(&p, &removed, entries.len().max(1)), &entries, None)?;
        tx.commit()?;
        Ok(tally(&results, action_id))
    }

    /// Newest first.
    pub fn image_annotation_history(&self, limit: u32) -> Result<Vec<ActionSummary>> {
        if limit == 0 || i64::from(limit) > MAX_HISTORY {
            return Err(E::InvalidArgument);
        }
        let mut s = self.db.prepare("SELECT id,kind,label,state,created_at,items FROM image_annotation_actions ORDER BY seq DESC LIMIT ?1")?;
        let rows = s.query_map([limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get::<_, String>(3)?, r.get(4)?, r.get(5)?))
        })?;
        let mut out = vec![];
        for row in rows {
            let (id, kind, label, state, created_at, items) = row?;
            out.push(ActionSummary {
                id,
                kind: serde_json::from_value(kind.into())?,
                label,
                state: serde_json::from_value(state.into())?,
                created_at,
                items,
            });
        }
        Ok(out)
    }

    /// Reverses an action item by item. An image changed after the action keeps
    /// its newer state and is reported as a `Conflict`; undo is not repeatable.
    pub fn undo_image_annotation_action(&mut self, action_id: &str) -> Result<BulkResult> {
        let tx = self.immediate()?;
        let row: Option<(String, String, Option<String>)> = tx
            .query_row("SELECT state,entries,alias FROM image_annotation_actions WHERE id=?1", [action_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((state, entries, alias)) = row else { return Err(E::NotFound) };
        if state != "applied" {
            return Err(E::Conflict);
        }
        let entries: Vec<Entry> = serde_json::from_str(&entries)?;
        let mut results = vec![];
        for e in &entries {
            let current = match editable(&tx, &e.id) {
                Err(outcome) => {
                    results.push(ItemResult { id: e.id.clone(), outcome, revision: None });
                    continue;
                }
                Ok(a) => a,
            };
            if current.revision != e.after {
                results.push(ItemResult { id: e.id.clone(), outcome: ItemOutcome::Conflict, revision: Some(current.revision) });
                continue;
            }
            let next = write(&tx, &e.id, &current, revert(&current, e))?;
            results.push(ItemResult { id: e.id.clone(), outcome: ItemOutcome::Applied, revision: Some(next.revision) });
        }
        if let Some(alias) = alias {
            let change: AliasChange = serde_json::from_str(&alias)?;
            if let Some((key, created)) = &change.created {
                tx.execute("DELETE FROM image_tag_aliases WHERE alias_key=?1 AND canonical=?2", params![key, created.canonical])?;
                for (key, previous) in &change.repointed {
                    tx.execute("UPDATE image_tag_aliases SET canonical=?2 WHERE alias_key=?1 AND canonical=?3", params![key, previous, created.canonical])?;
                }
            }
        }
        tx.execute("UPDATE image_annotation_actions SET state='undone' WHERE id=?1", [action_id])?;
        tx.commit()?;
        Ok(tally(&results, Some(action_id.into())))
    }

    /// Distinct image tags in scope with counts (by case-insensitive identity),
    /// most used first, plus the library-wide aliases.
    pub fn image_tags(&self, source_id: Option<&str>) -> Result<TagList> {
        let mut counts: HashMap<String, (String, u32)> = HashMap::new();
        {
            let mut s = self.db.prepare(
                "SELECT j.value,count(*) FROM asset_registry r JOIN image_assets i ON i.id=r.id JOIN sources s ON s.id=r.source_id,
                 json_each(i.annotations,'$.tags') j WHERE r.kind='image' AND (?1 IS NULL OR r.source_id=?1)
                 AND (s.scope='folder' OR EXISTS(SELECT 1 FROM source_files f WHERE f.source_id=r.source_id AND f.relative_path=r.relative_path))
                 GROUP BY j.value",
            )?;
            let rows = s.query_map([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
            for row in rows {
                let (tag, n) = row?;
                let e = counts.entry(tag_key(&tag)).or_insert((tag, 0));
                e.1 += n;
            }
        }
        let mut tags: Vec<TagCount> = counts.into_values().map(|(tag, count)| TagCount { tag, count }).collect();
        tags.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| tag_key(&a.tag).cmp(&tag_key(&b.tag))));
        let truncated = tags.len() > MAX_TAG_LIST;
        tags.truncate(MAX_TAG_LIST);
        let mut s = self.db.prepare("SELECT alias,canonical FROM image_tag_aliases ORDER BY alias_key")?;
        let aliases = s.query_map([], |r| Ok(TagAlias { alias: r.get(0)?, canonical: r.get(1)? }))?.collect::<std::result::Result<_, _>>()?;
        Ok(TagList { tags, truncated, aliases })
    }

    fn rename_plan(db: &Connection, from: &str, to: &str, source_id: Option<&str>) -> Result<(String, String, Vec<(String, std::result::Result<Annotations, ItemOutcome>)>)> {
        let from = display_tag(from)?.ok_or(E::InvalidArgument)?;
        let to = display_tag(to)?.ok_or(E::InvalidArgument)?;
        if from == to {
            return Err(E::InvalidArgument);
        }
        // Targets must be canonical tags, so aliases never chain.
        if aliases(db)?.contains_key(&tag_key(&to)) && tag_key(&to) != tag_key(&from) {
            return Err(E::InvalidArgument);
        }
        let key = tag_key(&from);
        let mut s = db.prepare(
            "SELECT r.id,j.value FROM asset_registry r JOIN image_assets i ON i.id=r.id, json_each(i.annotations,'$.tags') j
             WHERE r.kind='image' AND (?1 IS NULL OR r.source_id=?1)",
        )?;
        let rows = s.query_map([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut ids = vec![];
        let mut seen = HashSet::new();
        for row in rows {
            let (id, tag) = row?;
            if tag_key(&tag) == key && seen.insert(id.clone()) {
                ids.push(id);
            }
        }
        if ids.len() > MAX_RENAME {
            return Err(E::LimitExceeded);
        }
        ids.sort();
        let items = ids.into_iter().map(|id| {
            let state = editable(db, &id);
            (id, state)
        });
        // Out-of-scope images are not counted or disclosed.
        let items = items.filter(|(_, s)| !matches!(s, Err(ItemOutcome::PermissionDenied | ItemOutcome::NotFound))).collect();
        Ok((from, to, items))
    }

    pub fn preview_image_tag_rename(&self, from: &str, to: &str, source_id: Option<&str>) -> Result<RenamePreview> {
        let (from, to, items) = Self::rename_plan(&self.db, from, to, source_id)?;
        let to_key = tag_key(&to);
        let (mut images, mut merged, mut unavailable) = (0, 0, 0);
        for (_, state) in &items {
            match state {
                Ok(a) => {
                    images += 1;
                    merged += u32::from(tag_key(&from) != to_key && a.tags.iter().any(|t| tag_key(t) == to_key));
                }
                Err(_) => unavailable += 1,
            }
        }
        let mut s = self.db.prepare("SELECT tags FROM annotations")?;
        let key = tag_key(&from);
        let mut audio_unchanged = 0;
        for tags in s.query_map([], |r| r.get::<_, String>(0))? {
            let tags: Vec<String> = serde_json::from_str(&tags?).unwrap_or_default();
            audio_unchanged += u32::from(tags.iter().any(|t| tag_key(t) == key));
        }
        Ok(RenamePreview { from, to, images, merged, unavailable, audio_unchanged })
    }

    /// Renames (or respells) an image tag in one transaction. With `keep_alias`
    /// (library-wide renames only), the old name becomes an alias of the new one
    /// and existing aliases of the old name follow it; undo reverses both.
    pub fn rename_image_tag(&mut self, from: &str, to: &str, source_id: Option<&str>, keep_alias: bool) -> Result<BulkResult> {
        let tx = self.immediate()?;
        let (from, to, items) = Self::rename_plan(&tx, from, to, source_id)?;
        let (from_key, to_key) = (tag_key(&from), tag_key(&to));
        if keep_alias && (source_id.is_some() || from_key == to_key) {
            return Err(E::InvalidArgument);
        }
        let mut alias = AliasChange::default();
        if keep_alias {
            let count: i64 = tx.query_row("SELECT count(*) FROM image_tag_aliases", [], |r| r.get(0))?;
            if count >= MAX_ALIASES {
                return Err(E::LimitExceeded);
            }
            let mut s = tx.prepare("SELECT alias_key,canonical FROM image_tag_aliases")?;
            let existing: Vec<(String, String)> = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
            drop(s);
            for (key, canonical) in existing {
                if tag_key(&canonical) == from_key {
                    tx.execute("UPDATE image_tag_aliases SET canonical=?2 WHERE alias_key=?1", params![key, to])?;
                    alias.repointed.push((key, canonical));
                }
            }
            tx.execute(
                "INSERT INTO image_tag_aliases(alias_key,alias,canonical,created_at) VALUES(?1,?2,?3,?4)",
                params![from_key, from, to, crate::jobs::now_secs()],
            )?;
            alias.created = Some((from_key.clone(), TagAlias { alias: from.clone(), canonical: to.clone() }));
        }
        let (mut results, mut entries) = (vec![], vec![]);
        for (id, state) in items {
            let current = match state {
                Err(outcome) => {
                    results.push(ItemResult { id, outcome, revision: None });
                    continue;
                }
                Ok(a) => a,
            };
            // Replace in place to keep the tag's position; merge if `to` is already present.
            let mut next = current.clone();
            let mut entry = Entry { id: id.clone(), after: 0, added: vec![], removed: vec![], comment: None, favorite: None, rating: None };
            let has_to = from_key != to_key && current.tags.iter().any(|t| tag_key(t) == to_key);
            let mut tags = vec![];
            for (i, t) in current.tags.iter().enumerate() {
                if tag_key(t) == from_key {
                    entry.removed.push((i, t.clone()));
                    if !has_to {
                        tags.push(to.clone());
                    }
                } else {
                    tags.push(t.clone());
                }
            }
            if !has_to {
                entry.added.push(to.clone());
            }
            next.tags = tags;
            let next = write(&tx, &id, &current, next)?;
            entry.after = next.revision;
            entries.push(entry);
            results.push(ItemResult { id, outcome: ItemOutcome::Applied, revision: Some(next.revision) });
        }
        let scope = if source_id.is_some() { " in one source" } else { "" };
        let text = truncate(format!(
            "Rename “{from}” to “{to}”{scope}{} · {} image{}",
            if keep_alias { ", keeping an alias" } else { "" },
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        ));
        let action_id = record(&tx, ActionKind::Rename, &text, &entries, keep_alias.then_some(&alias))?;
        tx.commit()?;
        Ok(tally(&results, action_id))
    }

    /// Removes an alias; tags already on images are unaffected.
    pub fn remove_image_tag_alias(&mut self, alias: &str) -> Result<()> {
        let display = display_tag(alias)?.ok_or(E::InvalidArgument)?;
        let removed = self.db.execute("DELETE FROM image_tag_aliases WHERE alias_key=?1", [tag_key(&display)])?;
        if removed == 0 {
            return Err(E::NotFound);
        }
        Ok(())
    }
}

/// Restore-time validation of the SS-046 tables (aliases canonical, bounded history).
pub(crate) fn validate_annotation_tables(db: &Connection) -> crate::Result<()> {
    let bad = || crate::invalid("Backup image annotation history or aliases are inconsistent");
    let mut s = db.prepare("SELECT alias_key,alias,canonical FROM image_tag_aliases")?;
    let rows: Vec<(String, String, String)> = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
    if rows.len() as i64 > MAX_ALIASES {
        return Err(bad());
    }
    let keys: HashSet<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    for (key, alias, canonical) in &rows {
        let ok = display_tag(alias).ok().flatten().as_deref() == Some(alias.as_str())
            && display_tag(canonical).ok().flatten().as_deref() == Some(canonical.as_str())
            && *key == tag_key(alias)
            && tag_key(canonical) != *key
            && !keys.contains(tag_key(canonical).as_str());
        if !ok {
            return Err(bad());
        }
    }
    let mut s = db.prepare("SELECT entries,alias FROM image_annotation_actions")?;
    let rows: Vec<(String, Option<String>)> = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
    if rows.len() as i64 > MAX_HISTORY {
        return Err(bad());
    }
    for (entries, alias) in rows {
        let entries: Vec<Entry> = serde_json::from_str(&entries).map_err(|_| bad())?;
        if entries.len() > MAX_RENAME || alias.map(|a| serde_json::from_str::<AliasChange>(&a)).transpose().is_err() {
            return Err(bad());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Source;

    struct Fx {
        dir: tempfile::TempDir,
        c: Catalog,
        source: Source,
    }
    fn fixture() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("a");
        std::fs::create_dir_all(&root).unwrap();
        let c = Catalog::open(&dir.path().join("db.sqlite")).unwrap();
        let source = c.add_source(&root).unwrap();
        Fx { dir, c, source }
    }
    impl Fx {
        fn image(&mut self, path: &str) -> String {
            self.c.register_image(&self.source, path, Some(&blake3::hash(path.as_bytes()).to_hex())).unwrap().header.id
        }
        fn notes(&self, id: &str) -> Annotations {
            self.c.image(id, None).unwrap().annotations
        }
        fn reopen(&self) -> Catalog {
            Catalog::open(&self.dir.path().join("db.sqlite")).unwrap()
        }
    }
    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }
    fn add(t: &[&str]) -> AnnotationPatch {
        AnnotationPatch { add_tags: tags(t), ..Default::default() }
    }

    #[test]
    fn tags_normalize_like_audio_with_nfc_and_explicit_limits() {
        assert_eq!(display_tag("  brand_A \t summer ").unwrap().as_deref(), Some("brand A summer"));
        assert_eq!(display_tag(" \n _ ").unwrap(), None);
        // Decomposed and precomposed spellings are one tag, stored as NFC.
        assert_eq!(display_tag("Cafe\u{301}").unwrap().as_deref(), Some("Café"));
        assert_eq!(tag_key("CAFÉ"), tag_key("cafe\u{301}"));
        assert_eq!(display_tag(&"é".repeat(64)).unwrap().unwrap().chars().count(), 64);
        assert_eq!(display_tag(&"x".repeat(65)), Err(E::LimitExceeded));
        // 64 four-byte characters fit the contract's 256-byte cap exactly.
        assert!(display_tag(&"🦊".repeat(64)).unwrap().is_some());
        assert_eq!(display_tag("bell\u{7}"), Err(E::InvalidArgument));

        let mut f = fixture();
        let id = f.image("fox.png");
        let r = f.c.edit_image_annotations(&id, 1, &add(&["Fox", "fox", "Cafe\u{301}", "  "])).unwrap();
        assert_eq!(r.annotations.tags, ["Fox", "Café"]);
        assert_eq!(r.annotations.revision, 2);
        // A different case of an existing tag is not a change; removal ignores case and form.
        let same = f.c.edit_image_annotations(&id, 2, &add(&["FOX"])).unwrap();
        assert_eq!((same.action_id, same.annotations.revision), (None, 2));
        let r = f.c.edit_image_annotations(&id, 2, &AnnotationPatch { remove_tags: tags(&["CAFÉ"]), ..Default::default() }).unwrap();
        assert_eq!(r.annotations.tags, ["Fox"]);
        let many: Vec<String> = (0..64).map(|i| format!("t{i}")).collect();
        let over = AnnotationPatch { add_tags: many, ..Default::default() };
        assert_eq!(f.c.edit_image_annotations(&id, 3, &over), Err(E::LimitExceeded));
        assert_eq!(f.notes(&id).tags, ["Fox"], "a rejected edit changes nothing");
        assert_eq!(f.c.edit_image_annotations(&id, 3, &AnnotationPatch { add_tags: tags(&["x"]), remove_tags: tags(&["X"]), ..Default::default() }), Err(E::InvalidArgument));
    }

    #[test]
    fn ratings_notes_and_favorites_are_explicit_plain_values() {
        let patch = |json: &str| serde_json::from_str::<AnnotationPatch>(json).unwrap();
        assert_eq!(patch("{}").rating, None, "absent keeps the rating");
        assert_eq!(patch(r#"{"rating":null}"#).rating, Some(None), "null clears it");
        assert_eq!(patch(r#"{"rating":4}"#).rating, Some(Some(4)));
        assert!(serde_json::from_str::<AnnotationPatch>(r#"{"tags":["x"]}"#).is_err(), "no full-array replacement field");

        let mut f = fixture();
        let id = f.image("a.png");
        for bad in [0, 6] {
            assert_eq!(f.c.edit_image_annotations(&id, 1, &patch(&format!(r#"{{"rating":{bad}}}"#))), Err(E::InvalidArgument));
        }
        let xss = "<img src=x onerror=alert(1)>\n<script>alert('n')</script> & \"quotes\"";
        let r = f.c.edit_image_annotations(&id, 1, &AnnotationPatch { comment: Some(xss.into()), favorite: Some(true), rating: Some(Some(5)), ..Default::default() }).unwrap();
        assert_eq!((r.annotations.comment.as_str(), r.annotations.favorite, r.annotations.rating), (xss, true, Some(5)));
        let r = f.c.edit_image_annotations(&id, 2, &patch(r#"{"rating":null}"#)).unwrap();
        assert_eq!(r.annotations.rating, None);
        assert_eq!(f.notes(&id).comment, xss, "notes are stored verbatim, never sanitized into markup");
        let long = AnnotationPatch { comment: Some("é".repeat(MAX_TEXT_BYTES / 2 + 1)), ..Default::default() };
        assert_eq!(f.c.edit_image_annotations(&id, 3, &long), Err(E::LimitExceeded));
        let nul = AnnotationPatch { comment: Some("a\0b".into()), ..Default::default() };
        assert_eq!(f.c.edit_image_annotations(&id, 3, &nul), Err(E::InvalidArgument));
    }

    #[test]
    fn stale_editors_get_conflicts_and_concurrent_writers_never_lose_tags() {
        let mut f = fixture();
        let id = f.image("a.png");
        let mut other = f.reopen();
        other.edit_image_annotations(&id, 1, &add(&["from other"])).unwrap();
        assert_eq!(f.c.edit_image_annotations(&id, 1, &add(&["mine"])), Err(E::Conflict));
        let fresh = f.notes(&id);
        f.c.edit_image_annotations(&id, fresh.revision, &add(&["mine"])).unwrap();
        assert_eq!(f.notes(&id).tags, ["from other", "mine"]);

        // Eight connections race refresh-and-retry edits on one image.
        let path = f.dir.path().join("db.sqlite");
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let (path, id) = (path.clone(), id.clone());
                std::thread::spawn(move || {
                    let mut c = Catalog::open(&path).unwrap();
                    let mut conflicts = 0;
                    loop {
                        let rev = c.image(&id, None).unwrap().annotations.revision;
                        match c.edit_image_annotations(&id, rev, &add(&[&format!("writer {n}")])) {
                            Ok(_) => return conflicts,
                            Err(E::Conflict) => conflicts += 1,
                            Err(e) => panic!("{e:?}"),
                        }
                    }
                })
            })
            .collect();
        let conflicts: u32 = threads.into_iter().map(|t| t.join().unwrap()).sum();
        let notes = f.notes(&id);
        assert_eq!(notes.tags.len(), 10, "every writer's tag survives ({conflicts} conflicts retried)");
        assert_eq!(notes.revision, 3 + 8);
    }

    #[test]
    fn bulk_edits_preview_counts_and_report_each_item() {
        let mut f = fixture();
        let ids: Vec<String> = (0..4).map(|i| f.image(&format!("{i}.png"))).collect();
        f.c.edit_image_annotations(&ids[0], 1, &add(&["Brand A", "old"])).unwrap();
        let root = f.dir.path().join("b");
        std::fs::create_dir_all(&root).unwrap();
        let offline = f.c.add_source(&root).unwrap();
        let away = f.c.register_image(&offline, "away.png", None).unwrap().header.id;
        f.c.db.execute("UPDATE sources SET available=0 WHERE id=?1", [&offline.id]).unwrap();
        let unknown = "00000000-0000-4000-8000-00000000abcd".to_string();
        let mut all = ids.clone();
        all.extend([away.clone(), unknown.clone()]);
        let patch = AnnotationPatch { add_tags: tags(&["brand a", "Summer campaign"]), remove_tags: tags(&["OLD"]), rating: Some(Some(4)), ..Default::default() };
        let dirty = |c: &Catalog| c.db.query_row("SELECT dirty FROM source_catalog_state WHERE source_id=?1", [&f.source.id], |r| r.get::<_, i64>(0)).unwrap();
        let before = dirty(&f.c);
        let preview = f.c.preview_bulk_annotations(&all, &patch).unwrap();
        assert_eq!((preview.targets.len(), preview.changes, preview.unchanged), (4, 4, 0));
        // An existing alias spelling resolves to the stored display form.
        assert_eq!(preview.additions, [TagEffect { tag: "brand a".into(), count: 1 }, TagEffect { tag: "Summer campaign".into(), count: 0 }]);
        assert_eq!(preview.removals, [TagEffect { tag: "OLD".into(), count: 1 }]);
        let skipped: Vec<_> = preview.skipped.iter().map(|s| (s.id.as_str(), s.outcome)).collect();
        assert_eq!(skipped, [(away.as_str(), ItemOutcome::SourceUnavailable), (unknown.as_str(), ItemOutcome::NotFound)]);
        assert_eq!(dirty(&f.c), before, "preview writes nothing");

        // Another editor changes one item after the preview.
        f.reopen().edit_image_annotations(&ids[3], 1, &add(&["meanwhile"])).unwrap();
        let mut targets = preview.targets.clone();
        targets.push(BulkTarget { id: away.clone(), revision: 1 });
        let result = f.c.apply_bulk_annotations(&targets, &patch).unwrap();
        assert_eq!((result.applied, result.conflicts, result.skipped, result.unchanged), (3, 1, 1, 0));
        assert_eq!(f.notes(&ids[0]).tags, ["Brand A", "Summer campaign"]);
        assert_eq!(f.notes(&ids[1]).tags, ["brand a", "Summer campaign"]);
        assert_eq!(f.notes(&ids[3]).tags, ["meanwhile"], "the unseen change is not overwritten");
        assert_eq!(f.notes(&ids[1]).rating, Some(4));
        assert!(dirty(&f.c) > before, "edits mark the source snapshot dirty through the shared trigger");
        let history = f.c.image_annotation_history(10).unwrap();
        assert_eq!(history[0].label, "Add “brand a”, “Summer campaign”, remove “old”, rate 4/5 · 3 images");
        assert_eq!((history[0].kind, history[0].items), (ActionKind::Bulk, 3));

        assert_eq!(f.c.preview_bulk_annotations(&[], &patch), Err(E::InvalidArgument));
        let too_many: Vec<String> = (0..=MAX_BATCH).map(|i| format!("{i}")).collect();
        assert_eq!(f.c.preview_bulk_annotations(&too_many, &patch), Err(E::LimitExceeded));
        let notes = AnnotationPatch { comment: Some("x".into()), ..Default::default() };
        assert_eq!(f.c.apply_bulk_annotations(&preview.targets, &notes), Err(E::InvalidArgument), "no bulk note overwrite");
        let dup = vec![preview.targets[0].clone(), preview.targets[0].clone()];
        assert_eq!(f.c.apply_bulk_annotations(&dup, &patch), Err(E::InvalidArgument));
    }

    #[test]
    fn undo_restores_exact_prior_state_and_keeps_later_changes() {
        let mut f = fixture();
        let ids: Vec<String> = (0..3).map(|i| f.image(&format!("{i}.png"))).collect();
        for id in &ids {
            f.c.edit_image_annotations(id, 1, &add(&["keep", "drop", "tail"])).unwrap();
        }
        let before: Vec<_> = ids.iter().map(|id| f.notes(id)).collect();
        let patch = AnnotationPatch { add_tags: tags(&["new"]), remove_tags: tags(&["drop"]), favorite: Some(true), rating: Some(Some(2)), ..Default::default() };
        let preview = f.c.preview_bulk_annotations(&ids, &patch).unwrap();
        let action = f.c.apply_bulk_annotations(&preview.targets, &patch).unwrap().action_id.unwrap();
        let later = f.notes(&ids[2]);
        f.c.edit_image_annotations(&ids[2], later.revision, &add(&["later"])).unwrap();

        // Restart before undo: history is durable.
        let reopened = f.reopen();
        drop(std::mem::replace(&mut f.c, reopened));
        let undo = f.c.undo_image_annotation_action(&action).unwrap();
        assert_eq!((undo.applied, undo.conflicts), (2, 1));
        for i in 0..2 {
            let now = f.notes(&ids[i]);
            assert_eq!((&now.tags, now.favorite, now.rating), (&before[i].tags, false, None), "exact order restored");
            assert_eq!(now.revision, before[i].revision + 2, "undo is a new revision, not a rewind");
        }
        assert_eq!(f.notes(&ids[2]).tags, ["keep", "tail", "new", "later"], "a later change wins over undo");
        assert_eq!(f.c.undo_image_annotation_action(&action), Err(E::Conflict), "undo is not repeatable");
        let history = f.c.image_annotation_history(100).unwrap();
        assert_eq!(history.iter().find(|h| h.id == action).unwrap().state, ActionState::Undone);
        assert_eq!(history[0].state, ActionState::Applied, "the later edit is untouched");
        assert_eq!(history.len(), 1 + 1 + 3, "bulk, later edit and three single edits");
        assert_eq!(f.c.undo_image_annotation_action("00000000-0000-4000-8000-000000000000"), Err(E::NotFound));
    }

    #[test]
    fn history_is_bounded_and_pruned_oldest_first() {
        let mut f = fixture();
        let id = f.image("a.png");
        for n in 0..(MAX_HISTORY as u32 + 5) {
            f.c.edit_image_annotations(&id, n + 1, &AnnotationPatch { rating: Some(Some(1 + (n % 5) as u8)), ..Default::default() }).unwrap();
        }
        let count: i64 = f.c.db.query_row("SELECT count(*) FROM image_annotation_actions", [], |r| r.get(0)).unwrap();
        assert_eq!(count, MAX_HISTORY);
        assert_eq!(f.c.image_annotation_history(0), Err(E::InvalidArgument));
    }

    #[test]
    fn a_failure_mid_batch_rolls_back_every_item_and_its_history() {
        let mut f = fixture();
        let ids: Vec<String> = (0..5).map(|i| f.image(&format!("{i}.png"))).collect();
        let preview = f.c.preview_bulk_annotations(&ids, &add(&["batch"])).unwrap();
        // Storage fails while writing the fourth item (after three succeeded).
        f.c.db
            .execute_batch(&format!(
                "CREATE TEMP TRIGGER fail_mid AFTER UPDATE OF annotations ON image_assets WHEN NEW.id='{}' BEGIN SELECT RAISE(ABORT,'injected'); END;",
                ids[3]
            ))
            .unwrap();
        assert_eq!(f.c.apply_bulk_annotations(&preview.targets, &add(&["batch"])), Err(E::Internal));
        let reopened = f.reopen();
        drop(std::mem::replace(&mut f.c, reopened));
        for id in &ids {
            assert_eq!(f.notes(id).revision, 1, "no partial batch survives");
        }
        assert!(f.c.image_annotation_history(10).unwrap().is_empty());
        // The same tokens still apply once storage recovers.
        assert_eq!(f.c.apply_bulk_annotations(&preview.targets, &add(&["batch"])).unwrap().applied, 5);
    }

    #[test]
    fn rename_merges_in_place_keeps_aliases_and_leaves_audio_alone() {
        let mut f = fixture();
        let (a, b, c) = (f.image("a.png"), f.image("b.png"), f.image("c.png"));
        f.c.edit_image_annotations(&a, 1, &add(&["bg", "x"])).unwrap();
        f.c.edit_image_annotations(&b, 1, &add(&["x", "Background", "BG"])).unwrap();
        // An audio sound with the same tag.
        std::fs::write(std::path::Path::new(&f.source.root).join("s.wav"), b"riff").unwrap();
        let sound = f.c.register(&f.source, "s.wav", &"b".repeat(64)).unwrap();
        f.c.annotate(&sound, &tags(&["bg"]), "", false).unwrap();
        f.c.db.execute("INSERT INTO image_tag_aliases VALUES('backdrop','backdrop','bg',0)", []).unwrap();
        let before = (f.notes(&a), f.notes(&b));

        let preview = f.c.preview_image_tag_rename("BG", "background", None).unwrap();
        assert_eq!((preview.from.as_str(), preview.images, preview.merged, preview.unavailable, preview.audio_unchanged), ("BG", 2, 1, 0, 1));
        assert_eq!(f.c.rename_image_tag("bg", "backdrop", None, false), Err(E::InvalidArgument), "an alias is not a target");
        let other = f.dir.path().join("o");
        std::fs::create_dir_all(&other).unwrap();
        let scoped = f.c.add_source(&other).unwrap().id;
        assert_eq!(f.c.rename_image_tag("bg", "background", Some(&scoped), true), Err(E::InvalidArgument), "aliases are library-wide");

        let r = f.c.rename_image_tag("bg", "background", None, true).unwrap();
        assert_eq!(r.applied, 2);
        assert_eq!(f.notes(&a).tags, ["background", "x"], "renamed in place");
        assert_eq!(f.notes(&b).tags, ["x", "Background"], "merged into the existing spelling");
        assert_eq!(f.c.sound(&sound).unwrap().user_tags, ["bg"], "audio tags are unchanged");
        let aliases = f.c.image_tags(None).unwrap().aliases;
        assert_eq!(aliases, [TagAlias { alias: "backdrop".into(), canonical: "background".into() }, TagAlias { alias: "bg".into(), canonical: "background".into() }]);
        // New edits use the alias.
        let added = f.c.edit_image_annotations(&c, 1, &add(&["Bg", "backdrop"])).unwrap();
        assert_eq!(added.annotations.tags, ["background"]);

        let undo = f.c.undo_image_annotation_action(r.action_id.as_ref().unwrap()).unwrap();
        assert_eq!(undo.applied, 2);
        assert_eq!((f.notes(&a).tags, f.notes(&b).tags), (before.0.tags, before.1.tags));
        let aliases = f.c.image_tags(None).unwrap().aliases;
        assert_eq!(aliases, [TagAlias { alias: "backdrop".into(), canonical: "bg".into() }], "alias migration reversed");

        // Respelling is a rename to the same identity, without an alias.
        let r = f.c.rename_image_tag("x", "X", None, false).unwrap();
        assert_eq!((r.applied, f.notes(&a).tags), (2, tags(&["bg", "X"])));
        assert_eq!(f.c.rename_image_tag("x", "Y", None, false).unwrap().applied, 2);
        assert_eq!(f.c.rename_image_tag("absent", "y", None, false).unwrap().action_id, None);
        f.c.remove_image_tag_alias("Backdrop").unwrap();
        assert_eq!(f.c.remove_image_tag_alias("backdrop"), Err(E::NotFound));
        let list = f.c.image_tags(None).unwrap();
        // Counts merge spellings by identity (bg/BG, Background/background).
        assert_eq!(list.tags.iter().map(|t| (t.tag.as_str(), t.count)).collect::<Vec<_>>(), [("Background", 2), ("BG", 2), ("Y", 2)]);
    }

    #[test]
    fn replacement_reimport_and_duplicate_content_keep_independent_annotations() {
        let mut f = fixture();
        // A real original in the source folder; annotation work never touches it.
        let original = std::path::Path::new(&f.source.root).join("one.png");
        std::fs::write(&original, b"\x89PNG\r\n\x1a\n original bytes").unwrap();
        let before = blake3::hash(&std::fs::read(&original).unwrap());
        let hash = "c".repeat(64);
        let one = f.c.register_image(&f.source, "one.png", Some(&hash)).unwrap().header.id;
        let copy = f.c.register_image(&f.source, "copy.png", Some(&hash)).unwrap().header.id;
        let mine = f.c.edit_image_annotations(&one, 1, &AnnotationPatch { add_tags: tags(&["hero"]), comment: Some("approved".into()), favorite: Some(true), ..Default::default() }).unwrap().annotations;
        assert_eq!(f.notes(&copy).tags, Vec::<String>::new(), "same bytes, separate identity and annotations");
        // Same-path content replacement and an unchanged reimport keep user facts and revision.
        let replaced = f.c.register_image(&f.source, "one.png", Some(&"d".repeat(64))).unwrap();
        assert_eq!((replaced.header.id.as_str(), &replaced.annotations), (one.as_str(), &mine));
        let again = f.c.register_image(&f.source, "one.png", Some(&"d".repeat(64))).unwrap();
        assert_eq!(again.annotations, mine);
        let renamed = f.c.rename_image_tag("hero", "lead", None, true).unwrap();
        f.c.undo_image_annotation_action(&renamed.action_id.unwrap()).unwrap();
        assert_eq!(blake3::hash(&std::fs::read(&original).unwrap()), before, "original bytes are unchanged");
    }

    #[test]
    fn scope_rules_hide_unauthorized_images() {
        let mut f = fixture();
        let root = f.dir.path().join("files");
        std::fs::create_dir_all(&root).unwrap();
        let files = f.c.add_source(&root).unwrap();
        f.c.db.execute("UPDATE sources SET scope='files' WHERE id=?1", [&files.id]).unwrap();
        f.c.db.execute("INSERT INTO source_files(source_id,relative_path) VALUES(?1,'granted.png')", [&files.id]).unwrap();
        let files = f.c.source(&files.id).unwrap();
        let granted = f.c.register_image(&files, "granted.png", None).unwrap().header.id;
        f.c.edit_image_annotations(&granted, 1, &add(&["secret"])).unwrap();
        // The grant is withdrawn: the image is no longer editable, listed or renamed.
        f.c.db.execute("DELETE FROM source_files WHERE source_id=?1", [&files.id]).unwrap();
        assert_eq!(f.c.edit_image_annotations(&granted, 2, &add(&["x"])), Err(E::PermissionDenied));
        assert!(f.c.image_tags(None).unwrap().tags.is_empty());
        assert_eq!(f.c.preview_image_tag_rename("secret", "public", None).unwrap().images, 0);
        let sound_like = f.c.db.query_row("SELECT id FROM sources WHERE id=?1", [&f.source.id], |r| r.get::<_, String>(0)).unwrap();
        assert_eq!(f.c.edit_image_annotations(&sound_like, 1, &add(&["x"])), Err(E::NotFound), "IDs must be image assets");
    }

    #[test]
    fn aliases_block_audio_only_portable_export_until_mixed_catalogs() {
        let f = fixture();
        assert!(f.c.export_portable().is_ok());
        f.c.db.execute("INSERT INTO image_tag_aliases VALUES('bg','bg','background',0)", []).unwrap();
        assert!(f.c.export_portable().is_err(), "aliases are never silently dropped from an audio-only export");
    }

    #[test]
    fn restore_validation_rejects_alias_chains_and_corrupt_history() {
        let f = fixture();
        assert!(crate::image_catalog::validate_image_database(&f.c.db).is_ok());
        f.c.db.execute_batch("INSERT INTO image_tag_aliases VALUES('a','a','b',0); INSERT INTO image_tag_aliases VALUES('b','b','c',0);").unwrap();
        assert!(validate_annotation_tables(&f.c.db).is_err());
        f.c.db.execute_batch("DELETE FROM image_tag_aliases; INSERT INTO image_tag_aliases VALUES('A','A','b',0);").unwrap();
        assert!(validate_annotation_tables(&f.c.db).is_err(), "keys must be normalized");
        f.c.db.execute_batch("DELETE FROM image_tag_aliases; INSERT INTO image_annotation_actions(id,kind,label,state,created_at,items,entries) VALUES('00000000-0000-4000-8000-000000000001','edit','x','applied',0,1,'[{\"id\":1}]');").unwrap();
        assert!(validate_annotation_tables(&f.c.db).is_err());
    }

    /// Synthetic rows (no media) carrying `tags`, inserted in one statement.
    fn synthetic(c: &Catalog, source: &str, n: u32, tags: &str) {
        c.db.execute(
            "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<?1)
             INSERT INTO asset_registry(id,kind,source_id,relative_path)
             SELECT printf('00000000-0000-4000-8000-%012d',i),'image',?2,printf('bulk/%06d.png',i) FROM n",
            params![n, source],
        )
        .unwrap();
        c.db.execute(
            &format!("INSERT INTO image_assets(id,technical_state,availability,enrichment_state,annotation_revision,annotations)
             SELECT id,'pending','available','disabled',1,'{{\"revision\":1,\"tags\":{tags},\"comment\":\"\",\"favorite\":false,\"rating\":null,\"caption\":null}}'
             FROM asset_registry WHERE kind='image' AND source_id=?1"),
            [source],
        )
        .unwrap();
    }
    #[test]
    fn bulk_and_rename_stay_bounded_at_their_limits() {
        let mut f = fixture();
        synthetic(&f.c, &f.source.id, MAX_RENAME as u32, r#"["shared","x"]"#);
        let ids: Vec<String> = (1..=MAX_BATCH).map(|i| format!("00000000-0000-4000-8000-{i:012}")).collect();
        let patch = AnnotationPatch { add_tags: tags(&["Brand A", "Summer campaign"]), remove_tags: tags(&["x"]), rating: Some(Some(3)), ..Default::default() };
        let start = std::time::Instant::now();
        let preview = f.c.preview_bulk_annotations(&ids, &patch).unwrap();
        let previewed = start.elapsed();
        let result = f.c.apply_bulk_annotations(&preview.targets, &patch).unwrap();
        let applied = start.elapsed() - previewed;
        assert_eq!(result.applied, MAX_BATCH as u32);
        let start = std::time::Instant::now();
        let renamed = f.c.rename_image_tag("shared", "Shared library", None, true).unwrap();
        let rename = start.elapsed();
        assert_eq!(renamed.applied, MAX_RENAME as u32);
        let bytes: Vec<i64> = f.c.db.prepare("SELECT length(entries) FROM image_annotation_actions ORDER BY seq").unwrap()
            .query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert!(bytes.iter().all(|b| *b <= 8 * 1024 * 1024), "history rows stay within the 8 MiB cap: {bytes:?}");
        let start = std::time::Instant::now();
        assert_eq!(f.c.undo_image_annotation_action(renamed.action_id.as_ref().unwrap()).unwrap().applied, MAX_RENAME as u32);
        let undo = start.elapsed();
        eprintln!("SS-046 bounds: preview {} items {previewed:?}, apply {applied:?}; rename {} images {rename:?}, undo {undo:?}; history bytes {bytes:?}", MAX_BATCH, MAX_RENAME);
        // One past the rename bound is refused rather than producing an unbounded history row.
        synthetic_more(&f.c, &f.source.id);
        assert_eq!(f.c.preview_image_tag_rename("shared", "y", None).map(|_| ()), Err(E::LimitExceeded));
        assert_eq!(f.c.rename_image_tag("shared", "y", None, false).map(|_| ()), Err(E::LimitExceeded));
    }
    fn synthetic_more(c: &Catalog, source: &str) {
        c.db.execute("INSERT INTO asset_registry(id,kind,source_id,relative_path) VALUES('00000000-0000-4000-8000-999999999999','image',?1,'bulk/extra.png')", [source]).unwrap();
        c.db.execute("INSERT INTO image_assets(id,technical_state,availability,enrichment_state,annotation_revision,annotations) VALUES('00000000-0000-4000-8000-999999999999','pending','available','disabled',1,'{\"revision\":1,\"tags\":[\"shared\"],\"comment\":\"\",\"favorite\":false,\"rating\":null,\"caption\":null}')", []).unwrap();
    }

    fn v12_database(path: &std::path::Path) -> Connection {
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
        ] {
            db.execute_batch(schema).unwrap();
        }
        db.pragma_update(None, "user_version", 12).unwrap();
        db
    }
    #[test]
    fn genuine_v12_upgrade_adds_annotation_tables_with_a_backup() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v12.sqlite");
        let mut old = Catalog { db: v12_database(&path) };
        let source = old.add_source(d.path()).unwrap();
        let image = old.register_image(&source, "Old.png", Some(&"a".repeat(64))).unwrap();
        let mut notes = image.annotations.clone();
        notes.tags = vec!["kept".into()];
        let image = old.set_image_annotations(&image.header.id, Some(&"a".repeat(64)), &notes).unwrap();
        drop(old);
        let mut c = Catalog::open(&path).unwrap();
        assert_eq!(c.db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 13);
        assert_eq!(c.image(&image.header.id, None).unwrap(), image, "existing annotations are untouched");
        let r = c.edit_image_annotations(&image.header.id, 2, &add(&["new"])).unwrap();
        assert_eq!(r.annotations.tags, ["kept", "new"]);
        let backup = std::fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().path()).find(|p| p.to_string_lossy().contains("pre-v13-")).unwrap();
        let db = Connection::open(backup).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 12);
        assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE name='image_tag_aliases'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
    #[test]
    fn v13_collision_rolls_back_and_keeps_the_v12_catalog_usable() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("v12.sqlite");
        let db = v12_database(&path);
        db.execute_batch("CREATE TABLE image_tag_aliases(collision TEXT); INSERT INTO image_tag_aliases VALUES('retained');").unwrap();
        drop(db);
        assert!(Catalog::open(&path).is_err());
        let db = Connection::open(&path).unwrap();
        assert_eq!(db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0)).unwrap(), 12);
        assert_eq!(db.query_row("SELECT collision FROM image_tag_aliases", [], |r| r.get::<_, String>(0)).unwrap(), "retained");
        assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE name='image_annotation_actions'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }
}
