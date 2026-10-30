# Engineering sources

Primary documentation reviewed during implementation, 30 October 2026:

- [llama.cpp server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md):
  streaming chat completions, schema-constrained JSON, model image capabilities,
  offline mode, image/context/batch limits and CUDA layer offloading.
- [llama.cpp multimodal](https://github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md):
  vision GGUF + matching projector; GPU projector offload and supported model families.
- [Reference Q4 weights](https://huggingface.co/ggml-org/Qwen2.5-VL-7B-Instruct-GGUF/blob/main/Qwen2.5-VL-7B-Instruct-Q4_K_M.gguf):
  4.68 GB weight file. This is not peak RAM/VRAM, and no GPU benchmark was inferred from it.
- [PDFium thread safety](https://pypdfium2.readthedocs.io/en/stable/python_api.html):
  PDFium is not thread-safe. NoteGen serializes PDF operations in one worker.
- [Tesseract TSV](https://tesseract-ocr.github.io/tessdoc/Command-Line-Usage.html):
  word boxes, paragraphs, confidence, language selectors and page segmentation.
- [Qt QThread](https://doc.qt.io/qtforpython-6.8/PySide6/QtCore/QThread.html):
  queued signals, worker lifecycle and safe GUI updates.

APIs and model capabilities can change. A model's benchmark claims do not establish
NoteGen's end-to-end accuracy. See the hardware acceptance procedure in PLAN.md.
