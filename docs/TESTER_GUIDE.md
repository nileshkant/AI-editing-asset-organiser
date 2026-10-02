# CreativeShelf audio v1 tester guide

**Preparation only: no installer has passed qualification yet.** Do not distribute
a development binary as this release. The requested first-release platforms are
macOS, Windows and Linux. The provisional test targets are Apple Silicon/macOS
14.2+, Windows 11 x64 and Ubuntu 24.04 x64; these are test plans, not supported
platform claims. Intel Mac, other distributions/architectures require separate evidence.

When a candidate is approved, supply its download, checksum, publisher/signature,
exact version/commit, OS requirements, known defects and tested update instructions
along with this guide. No tester should need Node, Rust, an AI subscription or
separate media-tool setup. Verify that promise offline on a clean machine first.

## First session

1. Install the verified signed candidate, launch it offline, and note OS/CPU and
   app version. On Windows verify offline WebView2 availability; on Linux verify
   the WebKitGTK/audio dependency closure without package downloads. Stop and
   report any unmet runtime dependency. Do not bypass OS signature warnings.
2. Use a **small disposable sample folder you own**, containing varied WAV/MP3/
   FLAC examples and at least one unsupported/corrupt test file. Record original
   checksums. Import the folder. The app indexes originals in place; it does not
   copy recordings into its application package or database.
3. Add a distinctive tag, comment, favorite and saved clip. Search for a filename,
   typo, tag and measured property; check results and empty/no-match states.
4. Save the source catalog through Settings. Import the same unchanged source in
   a fresh library and verify metadata reuse. Rescan after adding a new sample.
   Remove one disposable audio file and reimport: only confirmed-missing metadata
   is pruned. Disconnect an external source: offline entries must be retained.
   Relink the folder and check annotations/clip revisions survived.
5. Listen through real speakers/headphones. Check play, pause/resume, seek, looping,
   clip endpoints, output selection/disconnection/reconnection, and playback while
   another import runs. Confirm the waveform matches what you hear. A mock player
   or successful decode does not count as audible playback.
6. Export an approved WAV and FLAC clip. Open both in an independent editor; compare
   duration/sample rate/channel layout and listen for unexpected fades/clipping.
   Keep originals unchanged. Test relocated sidecar handoff against EDITOR_HANDOFF.md.
7. Open Settings → Agent access, start MCP explicitly and pair a real client with
   selected sources. Try HTTP or Show installed bridge for a command client. Start
   read-only; confirm hidden sources and write/export/path requests are denied.
   Explicitly enable edits/exports, approve one destination in the app and complete
   search/get/clip/export/status. Revoke and verify subsequent requests fail while
   another client works. Stop/restart; old credentials must fail. Store credentials
   privately, never in feedback or screenshots.
8. Make a populated backup in Settings. Change an annotation, restore and restart;
   confirm it returns. Pause imports, resume, purge waveform cache and check rebuild.
   Reinstall/upgrade from the previous candidate with backup retained. Uninstall
   must retain the library and every source file unless the user explicitly removes them.

## Qualification session (release owner)

Use clean isolated machines and disposable data for full-disk, interrupted import/
export, crash/lock recovery and malformed inputs. Do not fill the user's real disk
or kill a populated production app. Test non-admin permissions, minimum OS,
external-drive loss, stale snapshot conflicts, offline reinstall, and a long session
with large libraries/mixed playback/import/MCP activity. Record timings, memory,
UI responsiveness and resource growth; repeat suspicious spikes and keep raw logs.
Test keyboard-only navigation, visible focus, screen reader labels and contrast.
Re-run dependency audits; resolve critical/high defects and the Linux GTK advisory
before qualifying Linux. Do not use happy-path CI as substitute evidence.

## Current capability boundaries

Audio import/search, manual annotations, waveforms, native playback, saved clips,
lossless export, portable metadata and scoped MCP are the baseline. AI sound-event
recognition is not enabled: measured tags such as level/pace do **not** identify
rain, footsteps or other events. Filename/user tags may identify sounds; do not
report them as AI recognition. Optional provider and semantic recognition work is
tracked in SS-014/015/016. Image assets, image elements and Remotion presets are
upcoming SS-032/033 and are not in this release. No automatic updater or tray daemon
is enabled. MCP access ends on Quit/Stop and credentials expire on restart.

## Feedback and recovery

Use [the feedback template](TESTER_FEEDBACK.md). Include version, OS/CPU, steps,
expected/observed result and how often it occurs. Settings provides a local redacted
support report; inspect it before sharing. Avoid original media, filenames/paths,
private annotations, credentials and raw logs unless specifically requested and
reviewed. Never share an MCP token or signing secret.

Keep backups outside the application install directory. A missing source is fixed
by reconnecting/relinking, not copying audio into the app. Rebuild waveforms through
Settings. Stop MCP to revoke all pairings. For database trouble preserve the live
file and backup; follow RECOVERY.md and use the matching app version for an older
backup. Avoid downgrading a migrated live database. Include the error and support
report in feedback; source audio must remain unchanged.
