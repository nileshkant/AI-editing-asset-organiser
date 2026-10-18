CreativeShelf is a free, open-source library for your sound effects and audio files. Point it at your folders, then search, preview, tag, favorite and clip sounds, all on your own computer.

**Private by design:** CreativeShelf works offline and never uploads your files. Your audio stays in the folders you choose and is never modified. Optional AI-agent access (MCP) is off until you turn it on, and even then it only listens on this computer.

## Download

Pick the file for your computer from **Assets** below:

| Your computer | Download |
|---|---|
| **Mac** with Apple Silicon (M1 or newer), macOS 14.2+ | `CreativeShelf_…_aarch64.dmg` |
| **Windows** 10/11 (64-bit) | `CreativeShelf_…_x64-setup.exe` |
| **Linux** (64-bit, Ubuntu 24.04 or similar) | `CreativeShelf_…_amd64.AppImage` or `…_amd64.deb` |

Intel Macs are not supported yet.

## First launch

CreativeShelf is an independent open-source app, distributed directly instead of through an app store. Your computer may ask you to confirm the first launch. You only need to do this once.

**Mac**
1. Open the `.dmg` and drag **CreativeShelf** into **Applications**.
2. Open CreativeShelf. If macOS says it can't verify the app, click **Done**.
3. Open **System Settings → Privacy & Security**, scroll down and click **Open Anyway** next to CreativeShelf, then confirm.

After this, CreativeShelf opens normally. [Apple's guide](https://support.apple.com/102445) explains the same steps.

**Windows**
1. Run the `-setup.exe` installer.
2. If "Windows protected your PC" appears, click **More info → Run anyway**.

**Linux**
- **AppImage:** make the file executable (`chmod +x CreativeShelf_*.AppImage`) and run it.
- **Debian/Ubuntu:** `sudo apt install ./CreativeShelf_*_amd64.deb`

## Check your download (optional)

Each `SHA256SUMS-*.txt` file lists the expected fingerprints of the downloads. Linux files are also signed with the CreativeShelf release key (`creativeshelf-linux-release-key.asc`, which is also kept in the repository at `release/linux-release-key.asc`). Its fingerprint is `2FB8 3BE9 E38E 1AA7 2E2D  A3D7 08C4 C1F4 239D 6311`:

```
gpg --import creativeshelf-linux-release-key.asc
gpg --verify CreativeShelf_*_amd64.AppImage.asc CreativeShelf_*_amd64.AppImage
```

## Good to know

- This is a **beta**: audio is fully supported, and images and presets are still in development. Back up important catalogs before upgrading.
- CreativeShelf saves a small `.creativeshelf` metadata folder inside each folder you add, so a library survives moving computers. Try it on a copy of a small folder first if you're unsure.
- There's no automatic updater yet. Download the newest version from this page to upgrade; your library is kept.

## Feedback

Found a bug or have an idea? Open an issue on GitHub with your operating system and the steps to reproduce it. Please don't attach private recordings or file paths. Security problems: see `SECURITY.md`.

The bundled FFmpeg audio tools are built from verified upstream source under the LGPL; that source is attached to every release.
