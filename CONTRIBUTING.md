# Developing NoteGen Rust

Use Rust 1.99.1 and Cargo.lock. Desktop, resource planning, consensus, native adapters,
evidence and Word export are separate modules in rust/src. The Python predecessor is
archived under legacy/python and is not used in current builds.

Install Tesseract (eng/osd), PDFium chromium/8125, and set NOTEGEN_PDFIUM for tests.
Linux desktop builds need fontconfig/OpenGL/X11/Wayland system dependencies; CI lists
them. Windows needs MSVC Build Tools.

~~~shell
cargo fmt --all --check
cargo clippy --locked --all-features --all-targets -j 2 -- -D warnings
cargo test --locked --all-features --all-targets -j 2
cargo build --locked --release --features desktop --bins
~~~

Complete the relevant tests before declaring an iteration finished. Use real raster
PDF/native OCR checks for scan changes. Adapters need real vendor protocol examples;
mocked JSON does not establish model quality. Preserve source, alternatives and numeric
conflicts. Do not synthesize average transcripts or claim calibrated accuracy without
evidence. Physical model/GPU/corpus validation must be identified separately.

Do not commit model weights, private scans, generated documents or native binaries.
Preserve dependency notices and source links when packaging. Large AI tests are opt-in.
