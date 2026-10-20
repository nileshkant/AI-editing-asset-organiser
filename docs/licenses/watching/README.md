# Folder-watching dependency notices

SS-047 adds `notify` 8.2.0 (pinned in `src-tauri/Cargo.toml` and `Cargo.lock`)
for opt-in folder watching in the desktop adapter only; the core has no watcher
dependency. Default features are disabled except `macos_fsevent`.

Shipped per platform, with the license texts copied from the exact downloaded
crate archives:

| Crate | Version | License | Platforms |
| --- | --- | --- | --- |
| notify | 8.2.0 | CC0-1.0 | all |
| notify-types | 2.1.0 | MIT (selected from MIT OR Apache-2.0) | all |
| fsevent-sys | 4.1.0 | MIT | macOS |
| inotify | 0.11.5 | ISC | Linux |
| inotify-sys | 0.1.8 | ISC | Linux |

Windows uses `ReadDirectoryChangesW` through `windows-sys` 0.60.2, which was
already in `Cargo.lock` for other dependencies.
`kqueue` 1.2.1 and `kqueue-sys` 1.1.2 (MIT) appear in `Cargo.lock` for BSD
targets only and are not compiled for any CreativeShelf release platform.
Every other dependency of these crates was already in the lockfile.
