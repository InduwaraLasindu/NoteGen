# Primary engineering sources

Checked 30 October 2026. Vendor benchmarks are not NoteGen measurements.

- Rust desktop: https://github.com/emilk/egui
- Native GPU rendering: https://github.com/gfx-rs/wgpu
- PDFium binaries: https://github.com/bblanchon/pdfium-binaries/releases/tag/chromium/8125
- Rust bindings: https://docs.rs/pdfium-render/0.9.4/pdfium_render/
- OCR: https://github.com/tesseract-ocr/tesseract and https://github.com/tesseract-ocr/tessdata
- Native inference: https://github.com/ggml-org/llama.cpp/releases/tag/b11998
- Multimodal protocols: https://github.com/ggml-org/llama.cpp/tree/master/tools/mtmd
- LightOnOCR-3 vendor/prompt documentation: https://huggingface.co/lightonai/LightOnOCR-3-1B and https://huggingface.co/lightonai/LightOnOCR-3-4B
- LightOn GGUF publisher actually exercised: https://huggingface.co/richardyoung/LightOnOCR-3-1B-GGUF
- GLM vendor and task prompts: https://huggingface.co/zai-org/GLM-OCR
- GLM runtime conversion: https://huggingface.co/ggml-org/GLM-OCR-GGUF
- Qwen vendor models: https://huggingface.co/Qwen/Qwen3.5-2B , https://huggingface.co/Qwen/Qwen3.5-4B , https://huggingface.co/Qwen/Qwen3.5-9B , https://huggingface.co/Qwen/Qwen3.8-27B
- Qwen GGUF publishers: https://huggingface.co/unsloth/Qwen3.5-2B-GGUF and https://huggingface.co/ggml-org/Qwen3.8-27B-GGUF

rust/downloads.json pins immutable revisions, file names, publishers, byte lengths and
original LFS SHA256. rust/catalog.json ranks/models/memory figures are curated planning
estimates, not accuracy percentages or measured peaks. New runtime/model releases need
protocol, corpus and hardware acceptance again.
