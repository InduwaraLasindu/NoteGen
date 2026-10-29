# NoteGen

Windows scanned PDF → editable Word, with local OCR and contextual vision AI.

Designed for a 32 GB RAM / 16 GB NVIDIA RTX system. Conversion never needs a cloud
service, account, API key, or internet connection after dependencies and model files
are installed. Models are separate downloads; they are not bundled in this repository.

## Build gates

1. Core contracts, safe rasterization, preprocessing, OCR, editable tables, DOCX export.
2. Local vision AI, page pipeline, bounded recovery, durable checkpoints.
3. Windows desktop interface, live activity, review and correction workflow.
4. End-to-end tests, Windows build workflow, documentation and hardware acceptance.

See [the implementation plan](docs/PLAN.md). Each stage is tested before proceeding.
Unreadable handwriting cannot be guaranteed; uncertain content must stay visible for review.
