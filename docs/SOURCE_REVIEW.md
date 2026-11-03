# Source review tools

These workflows preserve original scans. Changes use review revisions with undo
and redo. Output files use new names outside the private project data folder.
The implementation and its current test evidence are tracked separately in
[the feature ledger](FEATURES_0.5.0.md) and [testing guide](TESTING.md).

## Confirm masks before recognition

Choose the intended recognition settings first. In the desktop app, open
**Projects & Tools → Privacy → Prepare source pages before recognition**.
Inspect the prepared source in Review, select a source rectangle, then use
**Confirm selected source masks**. Opaque-cover suggestions require confirmation:
a dark rectangle can also be a legitimate figure.

Use **Recognize confirmed prepared pages** after checking the masks. Preparation
does not run OCR, orientation/script detection or AI. Recognition checks the
original input and saved recognition configuration; changed input or incompatible
settings require fresh preparation and confirmation.

```text
notegen-cli --settings selected.json tools redactions prepare scan.pdf --output-hint notes.docx
notegen-cli --settings selected.json tools redactions recognize-prepared PROJECT --output notes.docx
```

The preparation command prints `PROJECT`. Mask previews and selected proposal
indices can also be supplied through `tools redactions preview` and `confirm`.
Sharing copies use `share-preview` followed by `share` with its exact token.
Private original images, reader evidence and undo history remain private; inspect
every page of the newly built sharing copy before sending it.

## Languages, line boundaries and tables

Open **Projects & Tools → Recovery → Source geometry and language regions**.
Script suggestions are observations with native scores, not calibrated language
probabilities. Confirm a combined OCR language such as `eng+sin` for a paragraph
or line, preview its reading, then apply the reviewed preview. Required language
packs must already be installed. Geometry remains absent where a reader supplied
no precise word/cell locations.

Line repairs use ordered convex quadrilaterals in normalized page coordinates:

```json
[
  {"points":[{"x":0.1,"y":0.2},{"x":0.9,"y":0.18},{"x":0.9,"y":0.28},{"x":0.1,"y":0.3}]}
]
```

Save this as `lines.json`, then run:

```text
notegen-cli tools geometry repair-lines PROJECT 1 0 lines.json
```

The page ID is stable; block indices and line ordinals are zero based. A repaired
line can be re-read using the selected reader in the desktop app or a
`tools geometry reread-line PROJECT request.json` request:

```json
{"page":1,"block":0,"ordinal":0,"reader_id":"glm-ocr","task":"handwriting","image_edge":0,"context":""}
```

Only pixels inside the repaired line are supplied; the outside is white and
confirmed masks remain opaque. The reading is retained as evidence for review.

Table repairs have a preview and explicit old-to-new cell mappings. New split
cells start empty; combining cells joins the explicitly mapped values. A merge
that would hide a nonempty cell is rejected. Proposed grid lines appear over the
source before application. An evenly spaced draft is a proposal to adjust,
not measured geometry.

## Isolated desktop state and shortcuts

```text
NoteGen.exe --data-dir "C:\NoteGen-test-state" --renderer glow
```

The selected folder holds UI persistence and initial model, shared-object and
writer folders. Existing settings saved inside that isolated state retain their
explicit paths. Source and export destinations remain user-selected. The renderer
option supports native graphics compatibility testing.

Keyboard bindings are editable under **Privacy and readability → Keyboard
shortcuts**. Conflicting or reserved combinations are rejected. Review bindings
run only when the keyboard-review focus control is selected; text editors keep
their normal text shortcuts.
