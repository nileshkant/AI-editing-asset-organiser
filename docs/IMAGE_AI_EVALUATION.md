# Image AI evaluation and the qualified path (SS-050)

This ticket defines how image AI is asked, trusted, measured and offered. It
analyzes no user library. Code: `crates/core/src/image_ai.rs` (requests, output
validation, embeddings, feature gating), `crates/core/src/image_eval.rs` (corpus,
scoring, runner) and `crates/core/src/bin/creativeshelf-image-eval.rs` (CLI).
Inputs: `eval/image-ai/`.

## Product rule: optional, gated on a key

AI features are optional. Without a provider that passed **Test connection**,
each feature stays listed in Settings → AI providers but is disabled, and it
says what it needs ("Add an API key for …", "Run Test connection for …").
Real-model quality results are recorded as pending until someone runs the
evaluation with a key. They are never estimated. Without a key, the route for AI
tagging is an external AI client working through the MCP server, as for audio
(`sounds.annotate`). The image equivalent is the image MCP slice that follows
this ticket.

## The qualified path

| Capability | Path | Needs | Status |
| --- | --- | --- | --- |
| Image descriptions and suggested tags | One request per image to an image-capable provider (OpenAI, Gemini, Anthropic or an OpenAI-compatible server) with the fixed schema below | A profile whose test found **image input** and **structured output** supported | Implemented and gated; quality pending a key |
| Search by meaning | Embed each image's description text (caption, tags, transcribed text) and the query with a text embedding model; exact cosine search with a cut-off | Descriptions plus a profile whose test found **embeddings** supported (not Anthropic) | Implemented and gated; quality pending a key |
| Word search over descriptions | Lexical match over the same description text | Descriptions only | Measured against the oracle; real quality pending |
| Offline tagging | none | — | **Not offered.** No CPU-local captioner is qualified (R8) |

A direct text-image embedding model (CLIP family) would allow search without
captions, but none could be evaluated: model downloads are not reachable from
the development environment. SS-051 may qualify one as a local retrieval pack.
Until then, search by meaning goes through descriptions.

### Request and pixels (handoff to SS-052)

- Pixels come from the decoder worker's canonical output: oriented, sRGB,
  straight alpha. They are downscaled so the long edge is at most 1024 (never
  upscaled) and re-encoded as PNG with only `IHDR`, `IDAT` and `IEND` chunks, so
  no EXIF, location, text or color-profile metadata is sent. Recorded as
  `srgb-oriented-fit1024-png/v1`. Release-mode cost for a 24-megapixel image:
  median 167 ms, at most 211 ms, about 1 MB sent.
- Instructions are fixed. They say that text inside an image is content to
  transcribe, never instructions. No file names, paths, tags, notes or catalog
  data are sent.
- One request per image, never retried by the adapter. SS-052 owns consent,
  retries, budgets and billing uncertainty.

### Output schema `creativeshelf.image-insight/v1`

`{caption: string, tags: string[], text_in_image: string, uncertain: boolean}`,
with no other fields.

- The answer must be a JSON object with exactly these fields and types. Unknown
  fields are refused, because a model answer cannot add data such as URLs.
- Caption and transcribed text: whitespace is collapsed, other control
  characters are refused, and each is at most 1,000 characters.
- Tags: at most 32. Each is normalized like a user tag (NFC, underscores to
  spaces, at most 64 characters), and duplicates by identity are removed.
- An answer outside these bounds is refused as a whole (`MODEL_OUTPUT_INVALID`),
  never truncated.
- The output is untrusted data. It is never executed, followed or used to pick
  an endpoint.

### Evidence and confidence (R7)

- An answer becomes `Evidence` with origin `model`, review state `suggested` and
  `ModelRef` (provider, model, served version, preprocessing ID, schema ID).
- Confidence is `unavailable`. Providers do not return calibrated scores, and
  self-reported numbers are not requested. `calibrated` will be used only with a
  measured calibration table, and none exists.
- An `uncertain` answer keeps its caption and suggests no tags.

### Embedding spaces (T3)

- A space is identified by provider, model, served model version, dimension,
  L2 normalization and input recipe, for example
  `openai/text-embedding-x@version#1536/l2/insight-text/v1`.
- Vectors from another space or dimension are refused. Zero or non-finite
  vectors are refused.
- Search returns nothing below the similarity cut-off, so an out-of-domain query
  gets no results rather than the nearest noise.

## Evaluation corpus `creativeshelf-generated/v1`

- **Images:** 40 images drawn by `image_eval.rs` with integer-only drawing, so every
  platform writes byte-identical PNGs. The committed manifest
  `eval/image-ai/corpus-v1.json` lists each image's BLAKE3 digest, and a test
  regenerates the corpus and compares. License: CC0 (generated by this
  repository). No third-party or private media is committed.
- **Categories:** watercolor, collage, 3D render, screenshot, text,
  transparent background, multiple objects, unusual crops and sizes
  (1200×200, 200×900, cut off at the edge, a tiny object), deliberately
  ambiguous images, and a flat control set. Each has at least four images and
  at least one held-out image.
- **Photo, pending:** photographs cannot be generated. Add them as a separate
  manifest of consented images in the same format, kept outside the
  repository. The runner checks their digests exactly the same way.
- **Splits:** every third image is development (14); the rest is held out (26).
- **Labels:** gold tags are facts with accepted spellings. Tag precision counts
  only predicted tags in the evaluation vocabulary. Other tags are reported as
  unjudged, not counted as wrong.
- **Caption rubric:** required facts (any accepted spelling, whole words) and
  forbidden words. The rubric is lexical, so a negation such as "not a square"
  still counts as an error. This is conservative.
- **Retrieval:** 17 judged queries plus 2 out-of-domain queries where no result
  is correct.
- **Language:** English only, which is the shipping UI language. Other languages
  are not claimed.

## Thresholds (pre-registered)

`eval/image-ai/thresholds.json` was committed on its own (commit
`e5e5bfb`) before any real-model result existed:

| Measure (held-out) | Threshold |
| --- | --- |
| Tag precision | ≥ 0.80 |
| Tag recall | ≥ 0.60 |
| Caption required-fact coverage | ≥ 0.75 |
| Captions with a forbidden fact | ≤ 10% |
| Retrieval recall@5 | ≥ 0.80 |
| Retrieval MRR | ≥ 0.60, and at least 0.20 above file names alone |
| Out-of-domain queries with results | 0 |
| Failed requests | ≤ 2% |
| p95 latency per image | ≤ 20 s |
| Mean input tokens per image | ≤ 3,000 |

The embedding cut-off is chosen on the development split only: the highest
value that keeps 95% of the uncut development recall@5. An incomplete run (the
request allowance ran out) never passes.

## Results so far

| Run | Held-out result |
| --- | --- |
| File names only (no AI, measured) | recall@5 0.216, MRR 0.294, 0 out-of-domain results |
| Oracle answering from the gold labels (checks the runner and that thresholds are reachable; says nothing about any model) | tags P 1.00 / R 1.00; word search MRR 0.88; bag-of-words embedding MRR 0.82; passes |
| Real image model | **Pending: needs an API key** |

## Running the evaluation

```sh
cargo build -p soundshelf-core --bin creativeshelf-image-eval
target/debug/creativeshelf-image-eval corpus /tmp/cs-corpus
target/debug/creativeshelf-image-eval baseline /tmp/cs-corpus
OPENAI_API_KEY=... target/debug/creativeshelf-image-eval run /tmp/cs-corpus \
  --thresholds eval/image-ai/thresholds.json --max-requests 60 --out report.json \
  --kind openai --model <vision model> --key-env OPENAI_API_KEY \
  --embed-kind openai --embed-model <embedding model> --embed-key-env OPENAI_API_KEY
```

- The run contacts only the providers named on the command line, sends only
  images listed in the manifest with matching digests, and stops at
  `--max-requests`. That is one request per image plus one per 64 texts
  embedded.
- Keys are read from the named environment variables and never written to the
  report.
- The report records the manifest and thresholds digests, the served model
  version, the embedding space, the cut-off, usage, latency and a per-image
  outcome.
- Check the provider's current documentation, license and pricing when you run
  it, and record the exact model in the review. This document does not assume
  them.

## Handoffs

- **SS-051** installs local packs. There is no qualified local image model yet.
  A local text-image retrieval pack or captioner must pass this same runner on
  the held-out split before it is offered. Through an OpenAI-compatible local
  server it already plugs in via `--kind openai_compatible --allow-local`.
- **SS-052** schedules real analysis. It uses `prepare_pixels` on the worker's
  decoded pixels and `describe`, and it owns consent per source, retries and
  budgets.
- **SS-053** persists `Evidence` from `image_ai::evidence`.
- **SS-059** stores vectors per `EmbeddingSpace` and must re-embed when the
  space ID changes.
