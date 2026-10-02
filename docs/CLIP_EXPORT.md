# Export a clip into an editor

1. Select a sound, make a waveform selection, and choose **Save Clip**.
2. Under **Saved Clips**, expand **Export clip**. WAV / 24-bit PCM and the source sample rate are the defaults. Select FLAC or another sample rate if your editor/project requires it. Set fades in whole milliseconds; a fade cannot exceed the clip's length.
3. Choose **Choose destination and export**, then choose a new file in the native save dialog. Existing media and manifests are never overwritten. Choose another filename on collision.
4. Keep CreativeShelf open while the clip renders and verifies. **Cancel export** stops work and removes temporary output. Successful exports show the path and frame count and appear in library search.
5. Choose **Copy exported path**, or navigate to that path in your editor's ordinary Import Media / Add Audio dialog. Import the `.wav` or `.flac` file. The adjacent `.soundshelf.json` is a provenance sidecar; the audio itself requires no CreativeShelf plugin, service or network connection.

Close CreativeShelf after export and confirm your editor can still play the imported file. For a precision smoke test, choose a known 15-second region at 48 kHz: its same-rate export contains 720,000 frames, with an exclusive end. Rate conversion rounds the selected duration to the nearest output sample frame. Stored gain and channel policy apply; fades are applied before resampling.

If a source changes, rescan and explicitly rebind/review the selection before exporting. Existing rendered files remain usable. If CreativeShelf reports a catalog-indexing warning, retain the audio and import its destination folder to retry indexing. Following an interruption after publication, the same import restores search visibility without creating another copy.

Temporary staging sits beside the chosen output so publication stays on the same filesystem. This requires hard-link support; unsupported destinations fail without replacing anything. If a drive disconnects, reconnect it and restart CreativeShelf to retry cleanup. No original recordings are copied into diagnostics or sidecar manifests.
