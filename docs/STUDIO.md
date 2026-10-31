# NoteGen Studio 0.3

The main view remains a PDF drop area, quality presets, a writer selector, and conversion
controls. **Context & writers** opens Studio; **Advanced** still exposes the complete
validated settings JSON, hardware limits and appearance controls.

## Context and clarification

Before scanning, enter the document topic and a terminology/abbreviation glossary.
Enable **Ask me about unclear words, numbers and pictures** to receive up to 32 questions
before Word export (8 by default). Each card displays the actual source crop, current
reading and alternatives. Enter exact replacement text/cell content, explain a diagram,
add context, or skip. Closing the card continues with the answers already entered.
Cancelling a job interrupts the waiting worker and preserves checkpoints.

Literal answers are human edits in separate `.edits.json` files. Original model readings
remain available. Context-only answers are saved in `guidance.json` and can request a new
local vision pass. New AI text is offered as evidence/alternatives; it cannot overwrite
confirmed text. Diagram interpretation can improve on unedited pages. Questions not
answered in unattended CLI mode remain available in `questions.json` and Review's inbox.

In **Review → Label handwriting / context**, drag a tight rectangle around a word,
number, letter or picture. Save project context, or teach a selected writer the exact
visible label. **Re-read with context** uses the saved project notes plus current Studio
context. It requires a compatible structured vision reader and produces new evidence.
Save any draft before re-reading; export a new Word file after reviewing changes.

## Writer memory

Add/select named people in **Writers & fonts**. The local profile stores only confirmed
labels and normalized source crops. On subsequent scans, compatible structured vision
readers receive a separate reference-sheet image containing averaged English-letter
shapes and recent confirmed words. Specialist LightOn/GLM prompt protocols are preserved.
Variants/crops remain a single model family for consensus purposes.

This is persistent visual example memory, **not neural weight fine-tuning**. It cannot
promise to decode every writer or learn isolated letters from an unsegmented cursive
paragraph. Confirmed word crops help future reading; individually labelled letters also
build font coverage. Profile revision/content participates in recognition-cache identity;
changing a profile affects subsequent jobs. The running job keeps its original snapshot.

Rename people, edit writing notes, forget examples, delete profiles, or export/import a
writer ZIP. Import validates paths, metadata, decoded dimensions and size limits and
creates a new profile ID. Deleting a writer removes their stored examples. Documents
already exported or copied elsewhere are separate files.

## Personal font packs

Label isolated English letters/digits/punctuation with **one isolated glyph** enabled.
The coverage display shows which of A–Z, a–z and 0–9 are present. Font export averages up
to 32 aligned samples per character, traces ink boundaries and creates a real TrueType
font. Adjust family name, spacing and average ink threshold before exporting.

A font ZIP contains `Handwriting.ttf`, SVG glyphs, `coverage.png`, a missing-character
report, and the writer profile/examples. Install the TTF through Windows Fonts settings.
Only explicitly labelled characters are mapped; missing characters are not invented.
This is a print-style outline font. Cursive joining, baseline calibration and advanced
kerning are not inferred. Review and refine inconsistent examples for a better font.
The pack contains personal handwriting data; sharing it also shares those examples.

## Third-party local models and updates

**Model studio → Add a third-party reader** supports a compatible GGUF + matching visual
projector, or an already-running OpenAI-compatible vision service on literal loopback
HTTP (for example LM Studio/Ollama/vLLM). Select the structured, LightOn or GLM prompt
adapter. The service must accept `image_url` input at `/v1/chat/completions` and advertise
its exact alias through `/v1/models`. Text-only servers are insufficient.

Give related model variants the same family ID. Rank affects preference; the existing
RAM/VRAM admission policy still applies to managed readers. External services control
their own allocations. NoteGen cannot certify their GPU offload and excludes them when
**Require verified acceleration** is enabled. No arbitrary repository code is executed.

Optionally configure a Hugging Face owner/repository and exact GGUF filenames. **Check
publisher update** displays the immutable revision and pair size. **Download, verify &
activate** resumes partial downloads, validates lengths, SHA256 and GGUF headers, then
atomically changes the reader descriptor. Both files come from the same revision.
The prior descriptor/files remain for **Roll back**. Jobs and model activation are
mutually exclusive in the desktop; no model updates are silently applied during scanning.
New architectures still require support from the installed native runtime.

## Optional public lookup

In **Public knowledge**, opt in, type a public topic, and search. Only that explicit
query is sent to Wikipedia; pages, OCR text and writer examples are not uploaded.
Choose which sourced excerpts to attach as background for future scans. Retrieved
content is treated as untrusted context, never as proof of missing text or numbers.
Internet failure does not affect ordinary offline conversion. Model downloads/update
checks are separate explicit network operations.

## Useful CLI commands

```powershell
notegen-cli writers
notegen-cli add-writer "Ada"
notegen-cli teach WRITER_ID letter-r.png r --glyph
notegen-cli font-pack WRITER_ID -o Ada-font.zip --family "Ada handwriting"
notegen-cli export-writer WRITER_ID -o Ada-profile.zip
notegen-cli import-writer Ada-profile.zip
notegen-cli register-reader my-reader.json
notegen-cli check-update glm-ocr
notegen-cli update-model glm-ocr
notegen-cli questions PROJECT_DIRECTORY
notegen-cli clarify PROJECT_DIRECTORY answers.json
notegen-cli reconsider PROJECT_DIRECTORY
```

Use `--settings SETTINGS.json` to choose profile/model folders or context. `answers.json`
is an array of `{id,replacement,context,skip,learn,glyph}` objects using IDs from `questions`.
Unattended conversion does not wait for interactive answers.
