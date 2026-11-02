# NoteGen 0.5.0

The [verification record](TESTING.md) describes the source and installer release
gates. Published assets include their exact source, binary hashes and validation
results in `installer-manifest.json`.

The local workbench brings project search, metadata, templates, references and recovery
together with source-linked review. Saved revisions support undo, redo, comparison and
restore. Focused keyboard controls provide save, approve, next and defer actions;
optional auto-advance waits for a successful save. Manual annotation labels retain
their source bindings and visibly require review when the underlying block changes.

Import PDF, PNG, JPEG, supported multipage TIFF and ordered scan folders. Optional
scan tools retain originals while applying cleanup, perspective correction, manual
curved-page correction or facing-page splitting. Numeric and table checks flag
disagreements without rewriting values. Region rereads retain reader evidence.

Export Word, Markdown, HTML, LaTeX, Excel or searchable PDF. Word adds semantic styles,
editable equations for a bounded grammar and previewed correction import. Source
links, annotation provenance and uncertainty remain explicit. Large Word exports keep
all content and explain when correction import exceeds supported limits.

Other additions include verified optional OCR language packs, offline AI bundles,
writer coverage and quarantine tools, profile versions, font proofs, hardware trials,
resource-budget previews, inspectable diagnostics and Windows credential storage.
App update channels default to stable; previews and check deferrals are explicit.
Rendering/OCR overlap is an opt-in CLI feature with resource admission and sequential
fallback. Retention-managed sessions offer verified export and selected cleanup;
native/OS temporary data remains outside that control.

The [workbench guide](WORKBENCH.md) covers desktop and CLI workflows. The
[100-item ledger](FEATURES_0.5.0.md) records implemented, partial, planned and excluded
scope. Corpus collection and Windows signing were excluded by request. Broad
handwriting quality, target GPU performance, native-language review and the complete
Narrator audit remain separate acceptance work. The installer is unsigned.

See [validation](TESTING.md), [accessibility](ACCESSIBILITY.md) and
[installation](INSTALLER.md) for evidence and requirements.
