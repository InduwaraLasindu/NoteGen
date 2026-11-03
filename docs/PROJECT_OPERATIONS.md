# Organize private projects

Open **Project workbench → Organize pages**, choose an action and inspect its
preview before confirming. The command palette also finds this panel. Source
page slots are stable identifiers: page 7 remains slot 7 after moving it first.
Reorder changes active reading/export order. Remove archives a slot; it does not
delete its original raster, reader evidence, confirmed corrections or history.

Existing projects first need **Enable stable source identities**. This assigns
identities from the current verified rasters; it does not invent historical
identities. New conversion projects receive a catalog automatically.

Split a page left/right or top/bottom at a fraction of its source size. Merge
selected pages side by side or vertically. Originals remain archived. Preview
reports text withheld because it crosses a split or lacks sufficient geometry;
inspect the original before manually confirming a derived reading. A derived
page stores exact parent raster identities and crop/composition mappings. Insert
adds an explicitly chosen raster at a zero-based active position. Derived or
inserted pages require review.

Use **Inspect current page order and revision** before layout undo/redo. Text
corrections made later remain intact. A changed order, source, mask, lock or edit
checkpoint invalidates a stale preview.

## Command line

All commands below are under `notegen-cli tools project`. Preview output contains
the confirmation token. Apply recomputes the same bounded operation, checking
that token against current state; a saved JSON preview cannot supply executable
transaction data.

```powershell
notegen-cli tools project migrations preview-project PROJECT
notegen-cli tools project migrations apply-project PROJECT --confirmation-token TOKEN
notegen-cli tools project layout inspect PROJECT
notegen-cli tools project layout preview PROJECT operation.json
notegen-cli tools project layout apply PROJECT operation.json --confirmation-token TOKEN
notegen-cli tools project layout undo PROJECT --expected-revision 2
notegen-cli tools project layout redo PROJECT --expected-revision 1
```

Example operations (choose one per file):

```json
{"operation":"reorder","pages":[3,1,2]}
```

```json
{"operation":"remove","pages":[2]}
```

```json
{"operation":"insert","image":"scan.png","position":1}
```

```json
{"operation":"split","page":1,"axis":"vertical","at":0.5}
```

```json
{"operation":"merge","pages":[1,2],"axis":"horizontal"}
```

## Duplicate reuse and private revisions

Select the original input and explicit project folders, or use recent projects.
Compatibility requires the same input bytes, recognition policy/settings,
verified model/runtime/OCR executable identities and original active layout.
Legacy identities that cannot be proven and HTTP model aliases are not eligible
for exact reuse. Review requirements and confirmed corrections remain in the
existing project. Read-only comparison reports differences; when either project
has confirmed masks it withholds text from both sides.

```powershell
notegen-cli --settings settings.json tools project reuse preview INPUT --project PROJECT
notegen-cli --settings settings.json tools project reuse choose INPUT --project PROJECT --index 0 --choice reuse --confirmation-token TOKEN
notegen-cli --settings settings.json tools project reuse choose INPUT --project PROJECT --index 0 --choice separate-revision --target NEW_FOLDER --confirmation-token TOKEN
notegen-cli tools project reuse compare FIRST_PROJECT SECOND_PROJECT
```

A separate revision copies private originals, evidence, history and locks to a
new folder. It preserves earlier lineage records and publishes its manifest
last. It is a private working copy; use the redacted sharing export for sharing.

## Migrations and recovery

Only registered project/configuration transitions are supported. Preview lists
exact before/after file hashes. Apply retains byte-exact before-images and a
receipt in `.notegen-transactions`. Rollback checks that later private edits are
unchanged; conflicting recovery preserves the journal for inspection.

```powershell
notegen-cli tools project migrations preview-settings settings.json
notegen-cli tools project migrations apply-settings settings.json --confirmation-token TOKEN
notegen-cli tools project migrations preview-rollback PATH_TO_RECEIPT
notegen-cli tools project migrations rollback PATH_TO_RECEIPT --confirmation-token TOKEN
notegen-cli tools project migrations recover-project PROJECT
notegen-cli tools project migrations recover-settings settings.json
```

## Urgent queue inputs

An urgent input can yield a running lower-priority job after a page preparation,
reader/consensus or finalization checkpoint is saved. An active OCR/AI request
finishes before yielding, and native workers release their resources before the
urgent input starts. The interrupted input resumes automatically with its original
settings and output destination. Pausing the queue or answering a clarification
continues to take precedence. Ordinary cancellation remains a separate action.
