# Portable audio handoff

Every successful export supplies a WAV/FLAC and a `.soundshelf.json` sidecar.
Use the ordinary Import Media command in your editor. CreativeShelf need not
remain open after export. Resolve automation is unavailable in audio v1; manual
import remains available. Codec support depends on the target editor.

## Remotion audio

Export into an existing Remotion project's `public/audio/` folder, or move both
exported files there together. This is an explicit rendered clip, not a copy of
the source library. Run the utility from this repository (development form):

```sh
node scripts/cargo.mjs run -p soundshelf-core --bin creativeshelf-handoff -- \
  '/path/to/project/public/audio/rain.wav.soundshelf.json' \
  '/path/to/project/public' '/path/to/project/src/rain-props.json'
```

Use the actual sidecar path returned by export. The utility verifies BLAKE3
content hash, manifest version, sample timing, regular files and public-root
containment. It refuses existing output files. Build a standalone utility with
`node scripts/cargo.mjs build -p soundshelf-core --bin creativeshelf-handoff --release`.
It needs no installed editor, MCP session or source library. Props include a
public-relative `media` path and decimal-string `sampleFrames`; duration is
sampleFrames / sampleRate, not video-frame count.

In a Remotion 4 project, import the props JSON and use:

```tsx
import {Html5Audio, staticFile} from 'remotion';
import audio from './rain-props.json';

export const Sound = () => <Html5Audio src={staticFile(audio.media)} />;
```

The exported audio already contains the selected region, gain, fades and channel
mapping; do not apply those again. Composition duration is a separate video
setting: `Math.ceil(audio.durationSeconds * fps)` includes the entire clip. Keep
props and the public folder when relocating the project. Use `staticFile` on the
raw relative name; do not encode filenames again. See official
[staticFile](https://www.remotion.dev/docs/staticfile) and
[Html5Audio](https://www.remotion.dev/docs/html5-audio) documentation.

No Remotion package is added to the desktop app. This adapter covers audio file
handoff; it does not create presets, promise every codec works in every browser,
or certify a render in an untested Remotion project. WAV is the conservative
baseline; verify playback/render in your target project before delivery.

## Agent instructions

The distributable skill is `integrations/creativeshelf-audio/SKILL.md`. Copy its
folder into the skill location supported by your agent, then pair that agent in
CreativeShelf. Installation is deliberate; this repository does not modify your
agent configuration. The workflow searches, inspects, records listening and
license evidence, creates a reviewed selection and exports to a native-approved
destination. No shell commands are assembled from catalog metadata.
