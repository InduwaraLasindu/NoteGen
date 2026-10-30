# Third-party notices

NoteGen source is MIT-licensed. Runtime/dependency/model licenses apply independently.
Windows packages collect Cargo dependency licenses/metadata (including font notices),
PDFium license material, and the original Tesseract distribution notices. Native runtime
setup preserves upstream notices. Models are separate downloads.

| Component | Purpose / source |
|---|---|
| egui / eframe | Rust GUI · MIT / Apache-2.0 · https://github.com/emilk/egui |
| wgpu | GPU rendering · MIT / Apache-2.0 · https://github.com/gfx-rs/wgpu |
| PDFium | PDF rendering · BSD-style plus third-party terms · https://pdfium.googlesource.com/pdfium/ |
| pdfium-render | Rust bindings · MIT / Apache-2.0 · https://github.com/ajrcarey/pdfium-render |
| Tesseract | OCR · Apache-2.0; libraries have separate terms · https://github.com/tesseract-ocr/tesseract |
| llama.cpp | Native inference · MIT · https://github.com/ggml-org/llama.cpp |
| CUDA / cuBLAS | Optional NVIDIA runtime · NVIDIA terms · https://docs.nvidia.com/cuda/eula/index.html |
| LightOnOCR-3 | Specialist models · Apache-2.0 · https://huggingface.co/lightonai |
| GLM-OCR | Specialist model · see upstream terms · https://huggingface.co/zai-org/GLM-OCR |
| Qwen | Vision models · see each model's license · https://huggingface.co/Qwen |

Cargo.lock pins dependencies; packaged licenses include original available crate files.
Community GGUF conversions retain upstream model terms and publisher notices. Sources
and exact revisions are documented in docs/SOURCES.md and rust/downloads.json. Legacy
Python dependencies are preserved separately; they are not Rust runtime requirements.
