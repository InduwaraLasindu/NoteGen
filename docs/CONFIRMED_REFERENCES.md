# Footnotes and citations

Open a saved project in Review, select the block containing a visible marker,
then open **Project workbench → Recovery & references → Confirmed footnotes and
citations**. Enter the exact marker and its occurrence, select the target page
and block, and inspect the source marker and target body before confirming.
For citations, enter a confirmed title and optional author/year. These fields
are explicit metadata; the application does not infer them from a name or date.

Confirmation creates an ordinary review revision without rewriting the recognized
text. Undo/redo restores/removes the relationship. Remove a mistaken link before
confirming a replacement. Attached local background files are a separate feature;
they never become citations merely because their text resembles a marker.

Links bind exact source rasters, block hashes and UTF-8 marker ranges. Changed or
ambiguous text, missing/archived targets, overlapping imported links, stale source
images and masks prevent semantic linking. Export keeps the original marker/plain
text and reports the reason. Masked target bodies and bibliography metadata are
withheld. Every export rechecks reference state before publication.

## Export behavior

- Word creates real footnote parts and source-preserving custom markers. Confirmed
  citations produce CITATION fields, explicitly supplied bibliography metadata and
  a BIBLIOGRAPHY field. Their cached visible result retains the source marker/body.
- Markdown creates footnotes and anchored bibliography links.
- HTML creates note/citation links with backlinks.
- LaTeX creates escaped footnotes and bibliography entries. Recognized text is
  never executed as TeX instructions.
- Traceability JSON, including the graph embedded in spreadsheet exports, records
  the resolved source-bound relationships and unresolved-link warnings.

Semantic reference paragraphs and relocated note bodies are outside Word correction
import. Other supported text/table controls remain importable. A visible export
notice explains that boundary. Actual Word/TeX rendering and bibliography style
acceptance require their native applications; package/XML tests alone do not prove
their appearance in every editor.

## CLI

```powershell
notegen-cli tools project references inspect PROJECT
notegen-cli tools project references preview PROJECT 1 reference.json
notegen-cli tools project references confirm PROJECT 1 reference.json --confirmation-token TOKEN
notegen-cli tools project references remove PROJECT 1 LINK_ID
```

The request uses zero-based block indices and UTF-8 byte offsets. For example, a
marker `[1]` at bytes 6–9 in anchor block 0 can point to page 2, block 0:

```json
{
  "kind": "footnote",
  "anchor_block": 0,
  "marker_start": 6,
  "marker_end": 9,
  "target_page": 2,
  "target_block": 0,
  "bibliography": null
}
```

For a citation use `"kind":"citation"` and explicitly supply
`"bibliography":{"title":"Confirmed title","author":"Confirmed author","year":"2024"}`.
Empty author/year is permitted; a nonempty title is required. Inspect the returned
preview and token. Confirm recomputes the request and refuses a changed source,
marker, target, metadata or project revision.
