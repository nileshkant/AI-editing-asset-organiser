# Optional local model packs (SS-051)

A model pack is an optional download that lets a feature run on this computer.
CreativeShelf is complete without one: import, browsing, tags, search, the MCP
server and provider-based AI do not depend on any pack. Nothing is bundled.

## What is trusted

The allowlist is `crates/core/src/model_pack_manifest.json`, compiled into the
app. **It is empty today**, because no local model has passed the SS-050
evaluation (see [IMAGE_AI_EVALUATION.md](IMAGE_AI_EVALUATION.md)). Adding a pack
is a reviewed code change that must attach the held-out evaluation result. The
settings page says so instead of offering anything.

An entry carries: id and dotted-number version; the capability
(`image_description` or `image_embedding`); for embeddings a pinned
`embedding_space`; an HTTPS origin and the hosts a download may be served from;
license identifier and a notice text that is shown before download; requirements
(`os`, `arch`, `cpu_features`, `min_ram_bytes`, `needs_accelerator`, `runtime`);
and a flat list of files, each with exact byte length and BLAKE3 digest.
Unknown fields, unsafe paths (`..`, absolute, backslash, hidden, duplicates
including by case), non-HTTPS origins, credentials in URLs, non-standard ports
and missing notices are all rejected when the manifest is parsed.

There is no archive format. Files are fetched one by one and written under the
pack's own folder, so there is nothing to extract, no archive path or link to
traverse, and nothing downloaded is ever run.

## Lifecycle

States shown to the user: not installed, downloading, verifying, downloaded and
verified (not in use), active, failed, and incompatible. A pack that was
interrupted shows how much can be resumed.

- **Download.** One at a time. Free disk space is checked first (size, minus
  what is already downloaded, plus 5% and 64 MiB). Every request is HTTPS to an
  allowlisted host with the provider adapter's address policy (no private or
  local addresses). Redirects are followed by hand, at most three, only to
  another allowlisted HTTPS host. Files larger than 8 MiB are fetched in 8 MiB
  range slices, so a stalled connection ends within 90 s and cancel is
  responsive; a server without range support is streamed whole.
- **Resume.** Only when the part file's recorded URL, size and digest match,
  the server answers 206 with the expected total and range, and the ETag is
  unchanged. Otherwise the part is discarded and the file restarts, once.
  Wrong bytes are never kept; an interrupted prefix is.
- **Verify.** Each file is hashed as it streams and must match its trusted size
  and digest; extra or missing bytes fail. A second pass (the Verifying state)
  hashes the staged files again. The installed folder appears with one
  atomic rename only after every file has verified. Activation hashes all files
  again, so corruption after install is caught, and a pack that fails is removed.
- **Update.** A new version installs beside the working one. The working pack
  stays active until the new one is activated; a failed update changes nothing.
  The previous version stays installed, so rollback is activating it again.
- **Activate / deactivate.** Replacing or deactivating waits (10 s in the app)
  for running tasks to finish. New tasks are refused during that wait.
- **Uninstall.** Deletes only the pack's own folder (the path must be inside the
  store and not a link). State is saved first, so a crash leaves an orphan
  folder that is removed at the next start.
- **Startup.** No network. Missing or resized files mark the pack failed and
  deactivate it; unknown folders and staging for retired packs are removed;
  corrupt state is set aside as `state.json.unreadable`.

## Embedding spaces

Activating an embedding pack whose `embedding_space` differs from the active
one records a pending reindex and bumps the store `generation`. Vectors are
never mixed: SS-059 stores vectors per space and must treat a space it did not
write as absent. Packs that share a space switch without a reindex.
`acknowledge_reindex` clears the note once the owner of reindexing has
scheduled it. Annotations and exports are not touched.

## For inference code (SS-052, SS-059)

Call `ModelPacks::acquire(capability)` and keep the returned `Lease` for as long
as pack files are in use. It carries the pack identity, `space_id` and
`generation` to stamp on results, and `pack_dir` gives the folder. If the lease
`generation` no longer matches, the result belongs to a replaced model.

## Where state lives

`<data folder>/model-packs/` holds `state.json`, `installed/` and `staging/`.
It is deliberately not in the catalog: packs are device-local binaries, so a
catalog backup, restore, source snapshot or portable transfer must not claim a
pack exists on another computer. Losing it only means installing again.

## Limits

- No real model has been installed or run. T3 (real inference with the packaged
  runtime) is open until a pack is qualified and a runtime ships.
- RAM detection works on Linux and macOS; Windows reports it as unknown and the
  page says the check was skipped. Free-disk detection works on Unix only; on
  Windows the preflight is skipped and a full disk fails the write.
- No inference runtime ships yet (`System::runtimes` is empty), so every pack
  that names a runtime is reported incompatible until one does.
