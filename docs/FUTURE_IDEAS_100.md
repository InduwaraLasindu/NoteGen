NoteGen: 100 proposals for future releases

These are recommendations, not a record of implemented features. Items 1–10 are my highest priorities; the remaining items cover recognition, review, workflow, personalization, performance, setup, reliability, accessibility and exports.

1. **Representative handwriting benchmark.** Build a versioned, permission-cleared corpus of real notes, including difficult pages, to measure whether each release improves recognition.
2. **Accuracy scorecards.** Report character, word, exact-number and table-cell accuracy separately instead of hiding different failures behind one score.
3. **Calibrated uncertainty.** Use held-out examples to connect uncertainty labels with observed error rates and choose sensible abstention thresholds.
4. **Numeric integrity checks.** Flag suspicious decimals, signs, dates, units and repeated identifiers for review without silently rewriting them.
5. **Reader selection by region.** Choose readers using measured strengths for handwriting, tables, equations and figures rather than one selection for an entire document.
6. **Scan quality preflight.** Detect blur, clipping, glare, low contrast and tiny writing before conversion, with practical rescan suggestions.
7. **One-page trial run.** Let users test one representative page and inspect quality, runtime and settings before processing a large batch.
8. **Source-to-output traceability.** Make every output paragraph, table cell and figure explanation link back to its supporting scan region and reader evidence.
9. **Review undo and redo.** Provide dependable, multi-step undo and redo for text, cells, region edits, approvals and learned examples.
10. **Signed Windows releases.** Add Authenticode signing, trusted timestamps and signature verification to the release and update paths.
11. **Automatic script detection.** Suggest the writing system and language for each page, while allowing manual corrections.
12. **Mixed-language recognition.** Route different languages within the same page or paragraph to suitable readers and language resources.
13. **Multilingual resource packs.** Offer verified, optional OCR resources and tested profiles for additional languages, including Sinhala and Tamil.
14. **Right-to-left documents.** Preserve right-to-left text, mixed-direction numbers and table reading order through review and Word export.
15. **Editable mathematics.** Recognize equation structure and export editable equations or LaTeX with source-linked uncertainty.
16. **Scientific notation tools.** Handle chemical formulas, reactions, subscripts, superscripts and technical symbols with specialized validation.
17. **Code transcription.** Preserve indentation, punctuation, line numbers and monospaced formatting in handwritten or printed code.
18. **Reading-order editor.** Let users correct multi-column reading order by rearranging source regions before final export.
19. **Header and footer handling.** Detect recurring headers, footers and page numbers and let users keep, remove or map them to Word sections.
20. **Footnotes and references.** Link footnote markers, citations and bibliography entries while preserving ambiguous matches for review.
21. **Shadow and glare recovery.** Offer non-destructive cleanup variants for uneven lighting, shadows and shiny paper, with original-versus-cleaned comparison.
22. **Curved-page correction.** Flatten photographed book pages while retaining the original scan and warning about distorted regions.
23. **Perspective correction.** Provide an editable four-corner correction tool for photographed pages and whiteboards.
24. **Handwritten line segmentation.** Allow manual repair of line boundaries when crowded, slanted or overlapping handwriting confuses readers.
25. **Paper-background controls.** Separate ruled lines, grid paper and paper texture from ink without erasing meaningful marks.
26. **Annotation layers.** Distinguish original text from later highlights, marginal notes, stamps and teacher corrections.
27. **Redaction-aware recognition.** Detect covered or intentionally redacted areas and preserve them as redactions rather than guessing hidden text.
28. **Preprocessing comparison.** Run a small set of cleanup variants on a sample region and let the user choose the most faithful result.
29. **Double-page splitting.** Split facing-page scans, repair the gutter boundary and preserve the original page relationship.
30. **Page organization.** Reorder, insert, remove, split and merge project pages with reversible edits and stable source references.
31. **Targeted region re-reading.** Let users draw a region and choose its reader, resolution and context without repeating a full-page pass.
32. **Disagreement heatmap.** Overlay reader disagreements on the source so users can spot uncertain words, numbers and cells quickly.
33. **Impact-based review queue.** Prioritize ambiguous totals, names, dates and technical terms instead of treating every mismatch equally.
34. **Fast review controls.** Add keyboard-first approve, correct, defer and compare actions with optional automatic movement to the next issue.
35. **Revision timeline.** Keep named versions of recognition, user corrections and contextual re-reads, with straightforward restore and comparison.
36. **Context-change comparison.** Show exactly what changed after adding a glossary, writer example or clarification, including newly uncertain interpretations.
37. **Interactive table-grid repair.** Let users adjust row and column boundaries, split cells and fix merged cells directly over the source.
38. **Table consistency checks.** Check row lengths, column types and stated totals while presenting discrepancies as review suggestions.
39. **Cross-page consistency review.** Surface inconsistent names, abbreviations, units and repeated values across a project without forcing them to match.
40. **Reversible correction rules.** Apply confirmed replacement rules to selected pages with a preview, exclusions and one-step rollback.
41. **Project dashboard.** Organize work with names, folders, tags, status filters, recent activity and visible pending-review counts.
42. **Local full-text search.** Search recognized text across projects and jump directly to matching source regions and reviewed output.
43. **Linked project references.** Attach related diagrams, reference images and background documents locally and clearly separate them from transcription evidence.
44. **Project templates.** Save reusable combinations of subject context, reader policy, review rules and export styling.
45. **Queue priorities.** Support drag-to-reorder jobs, priority levels and urgent small jobs inserted safely between ongoing stages.
46. **Content-based duplicate detection.** Recognize the same document under different filenames and offer reuse, comparison or a separate revision.
47. **Watched import folders.** Monitor chosen folders for new scans, with explicit import rules, duplicate handling and a pause switch.
48. **Batch image capture.** Combine camera captures and image folders into page-ordered projects before recognition.
49. **More input formats.** Import JPEG, PNG and multipage TIFF directly, followed by optional HEIC support where a reliable decoder is available.
50. **Recovery dashboard.** After an interrupted session, explain completed stages, reusable checkpoints, missing files and exactly what will run on resume.
51. **Example coverage coaching.** Show which letter forms, numbers and writing conditions a writer profile lacks, then suggest useful examples to collect.
52. **Writer-profile suggestions.** Offer local, opt-in matching to existing writer profiles, requiring confirmation before using the suggested profile.
53. **Writer-specific evaluation.** Measure a profile on held-out confirmed samples so personalization gains and regressions become visible.
54. **Confusion-pair exercises.** Generate focused review prompts for recurring ambiguities such as r/n, 1/l, 0/O and similar handwritten forms.
55. **Glossary suggestions.** Suggest possible project terms from confirmed corrections, with approval before adding them to recognition context.
56. **Profile version comparisons.** Compare writer-profile versions and safely branch a profile before making major changes.
57. **Bad-example quarantine.** Isolate questionable training examples with a reason and an easy restore path instead of immediately deleting them.
58. **Selective profile sharing.** Export only chosen examples and metadata, showing a clear privacy preview before creating the bundle.
59. **Richer handwriting fonts.** Extend font packs toward ligatures, alternate letter shapes and additional scripts using confirmed samples and appropriate shaping support.
60. **Font proof sheets.** Preview generated handwriting fonts on words, sentences and printable practice sheets, with missing-glyph warnings.
61. **Document-based hardware trials.** Offer an opt-in benchmark on a copied representative page, clearly separating speed measurements from quality results.
62. **Multi-GPU benchmarking.** Compare compatible devices with the same test and recommend a device using measured speed, memory use and stability.
63. **Measured memory budgets.** Record actual peak RAM and VRAM consumption and refine future admission estimates conservatively.
64. **Power and thermal profiles.** Provide quiet, battery, balanced and maximum-throughput modes with clear expectations and reversible settings.
65. **Resource sharing limits.** Let users reserve CPU capacity and GPU memory for other applications while NoteGen works in the background.
66. **Bounded warm-reader reuse.** Keep a recently used model resident across compatible jobs when memory permits, with an immediate release control.
67. **ETA confidence ranges.** Show an estimated finish window and explain which stages or unknowns are contributing most to uncertainty.
68. **Time-budget planning.** Estimate what a queue can finish within a chosen time budget and present explicit quality-versus-time options.
69. **Overlapped preparation.** Overlap bounded rendering and printed OCR with compatible work while keeping AI memory usage predictable.
70. **Fine-grained reconversion.** Reprocess only changed pages or regions and preserve unaffected recognition and confirmed edits.
71. **Exact installer preflight.** Show the selected readers, remaining download bytes, rollback space and disk requirements before AI setup begins.
72. **Model capability cards.** Explain each reader's supported tasks, tested languages, memory needs, license, publisher and measured limitations.
73. **Download scheduling.** Add bandwidth limits, metered-connection controls, planned download windows and a dedicated pause/resume manager.
74. **Offline AI bundles.** Export and import verified runtime/model bundles so another PC can be prepared without downloading everything again.
75. **Shared model storage.** Deduplicate identical weights and projectors by content hash, with safe version-aware references.
76. **Canary update validation.** Run a small known recognition test before activating a new model or runtime and retain the working version on failure.
77. **Per-project model pinning.** Resolve and restore the exact model revisions required by a project instead of relying on the current global selection.
78. **Update channels.** Offer stable and preview channels with compatibility notes, optional deferral and project-level version preferences.
79. **Safe storage cleanup.** Preview reclaimable space, distinguish cache from user data and protect active versions, pinned projects and rollback choices.
80. **Private-update credentials.** Provide a secure Windows credential-store interface for private release access instead of requiring an environment variable.
81. **Privacy dashboard.** Show active connections, update checks, optional lookups and local data locations in one understandable screen.
82. **Encrypted workspaces.** Offer optional encryption at rest with a carefully designed recovery-key and backup workflow.
83. **Confidential session mode.** Provide explicit temporary-workspace retention controls and a clear cleanup report after export.
84. **Share-ready redaction.** Create redacted copies of source regions, recognized text and project metadata before manual sharing.
85. **Inspectable diagnostic bundles.** Preview every included file and exclude document content by default when exporting support diagnostics.
86. **Plain-language troubleshooting.** Translate runtime, GPU, OCR and export failures into specific checks and recovery actions.
87. **Verified repair tools.** Repair damaged or missing model/runtime files selectively, preserving active registrations and confirmed project data.
88. **Migration recovery.** Add transactional project/configuration migrations with backups and a tested rollback path.
89. **Failure-injection testing.** Exercise disk-full, power-loss, disconnected-storage, GPU-loss and interrupted-update cases automatically.
90. **Stronger build provenance.** Add dependency inventories, reproducibility checks and verifiable build attestations alongside current checksums and manifests.
91. **Command palette and shortcuts.** Make major actions searchable and keyboard accessible, with configurable shortcuts and visible focus.
92. **Screen-reader support.** Audit accessible names, focus order, status announcements and native accessibility integration throughout the desktop.
93. **Readable-layout presets.** Test large text, high contrast and compact or spacious layouts without clipping important controls or explanations.
94. **Precision source magnifier.** Add a keyboard-accessible magnifier with synchronized source focus, region boundaries and clear disagreement markers.
95. **Interface localization.** Translate the application and help text independently of OCR language/model packs.
96. **Semantic Word styling.** Map recognized headings, lists, captions, equations and references to configurable Word styles or document templates.
97. **Additional text exports.** Export Markdown, HTML and LaTeX while carrying useful source references and uncertainty annotations.
98. **Searchable PDF export.** Create a source-preserving PDF with aligned searchable text and visible indications of unresolved regions.
99. **Spreadsheet table export.** Export reconstructed tables to Excel with source links, reviewed values and explicit warnings about uncertain cells.
100. **Word correction round-trip.** Import edits from an exported Word document back into a project, showing conflicts and preserving the original recognition history.

Design references: Microsoft's Windows accessibility checklist and SignTool documentation, and Hugging Face Hub's version-aware download/cache documentation. These are design references; the proposed implementation should retain NoteGen's native Rust application and local inference architecture.
