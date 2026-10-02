# Exact-artifact tester qualification

`release/qualification.json` is an explicitly pending manifest for the requested
macOS/Windows/Linux matrix. `npm run release:qualify -- manifest.json proof-directory`
checks that all fourteen required suites have observed pass results, human attribution,
bounded local proof files and unchanged SHA-256 hashes, and that artifact reports
agree with the same candidate commit, target and dependency locks. Missing/duplicate
suites, a partial platform matrix, unsigned artifacts, local/dirty build provenance, changed evidence and unresolved
critical/high defects fail closed. Medium/low defects require documented workarounds.

Each `stageReport`, `packageReport` and suite `evidence` is `{path, sha256}`, relative
to a dedicated ignored proof directory. Reports record artifact/installer checksums,
compressed/uncompressed size, runtime overhead, zero model packs, inventory and
platform signature verification. Reports come from SS-024 native inspection on each
platform; actual signed installer evidence remains pending. Use aliases rather
than tester personal information; no source recordings, credentials or database belong
in proofs. Keep raw private evidence locally and make a redacted review copy.

The validator verifies the **integrity and completeness of recorded evidence**.
It cannot prove a human actually heard audio, reviewed a notice, exercised a clean VM
or truthfully wrote an observation. Evidence must be independently reviewed. A pass
is not automatic publication or a production-ready claim. Synthetic validator fixtures
are marked synthetic and must never populate the real manifest.

See TESTER_GUIDE.md for the walkthrough and failure qualification. Preserve evidence
for the exact signed candidate; rebuilds or code/dependency/architecture changes require
new artifact reports and relevant requalification. SS-023's broader fuzz/corpus and
resource coverage, SS-024's all-platform installers/signatures and SS-034's Linux audit
remain pending. The fourteen suites include clean offline install, source references,
annotations/search, physical audio, independent editor, actual MCP clients, recovery,
disk-full/interruption, source retention, security, accessibility, long-session resources,
package/signature/size and tester documentation. No release decision is recorded yet.
