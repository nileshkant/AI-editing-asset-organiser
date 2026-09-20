# Release media tools

The production bundle must contain target-qualified `ffmpeg` and `ffprobe` binaries beside this file. They are intentionally not committed: each release build stages audited LGPL-compatible binaries, matching notices and source/build instructions here. Development builds may use `SOUNDSHELF_FFMPEG` and `SOUNDSHELF_FFPROBE`, or the known Homebrew locations on macOS.

Never copy arbitrary executables into a release. The release job must verify the binary architecture, version, license configuration, checksum and license notices before `tauri build`.
