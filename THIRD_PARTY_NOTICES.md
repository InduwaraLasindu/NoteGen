# Third-party components

NoteGen code is MIT-licensed. Dependency licenses continue to apply independently.
Python packages and separately installed model/runtime files are not relicensed by NoteGen.
Packaged distributions retain the dependencies' installed license files under `_internal`.

| Component | Purpose | License / upstream |
|---|---|---|
| PySide6 Essentials / Qt | Native desktop | LGPLv3 / GPLv3 / commercial; https://doc.qt.io/qtforpython-6/licenses.html |
| pypdfium2 | PDF rasterizer binding | Apache-2.0 or BSD-3-Clause; https://github.com/pypdfium2-team/pypdfium2 |
| PDFium | Native PDF renderer | BSD-style plus third-party notices; bundled by pypdfium2 |
| OpenCV | Scan preparation and grid detection | Apache-2.0; https://github.com/opencv/opencv |
| NumPy | Image arrays | BSD-3-Clause; https://numpy.org/doc/stable/license.html |
| Pillow | Image I/O and crops | HPND; https://github.com/python-pillow/Pillow |
| python-docx | Word generation | MIT; https://github.com/python-openxml/python-docx |
| lxml | DOCX XML | BSD-style; https://lxml.de/copyright.html |
| Pydantic | Page data validation | MIT; https://github.com/pydantic/pydantic |
| HTTPX | Local HTTP transport | BSD-3-Clause; https://github.com/encode/httpx |
| psutil | Memory telemetry | BSD-3-Clause; https://github.com/giampaolo/psutil |
| Tesseract (separate install) | Printed OCR | Apache-2.0; https://github.com/tesseract-ocr/tesseract |
| llama.cpp (separate install) | CUDA vision runtime | MIT; https://github.com/ggml-org/llama.cpp |
| Qwen2.5-VL-7B-Instruct (separate download) | Reference vision model | Apache-2.0; https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct |

Use an onedir distribution so Qt remains dynamically linked and its libraries can be
replaced. Source, build instructions and license notices are supplied in this repository.
Do not bundle weights or third-party executables without their required notices and terms.
PyInstaller uses GPL with a bootloader exception; it does not change NoteGen's license.
