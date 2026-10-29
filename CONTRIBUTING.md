# Development

Use Python 3.12 and the locked dependencies. Install Tesseract for the real OCR tests.

```shell
python -m pip install -r requirements-dev-lock.txt
python -m pip install --no-deps --no-build-isolation -e .
python -m ruff check src tests scripts
python -m ruff format --check src tests scripts
QT_QPA_PLATFORM=offscreen python -m pytest -q -rA
```

On PowerShell set `$env:QT_QPA_PLATFORM='offscreen'` before running pytest.

Keep PDFium on one worker and one AI request in flight. Preserve loopback-only
networking, disabled environment proxies, blocked redirects, explicit uncertainty,
atomic checkpoints and no implicit output overwrite. No documents, weights, logs,
credentials or private paths in git. Fixtures must be generated or permissively licensed.

Add meaningful regression evidence for behavior changes. Run the current iteration's
gate before moving on. Do not equate mocked AI tests with recognition accuracy tests.
Update TESTING.md when validating on Windows/CUDA. Report code + app/model/runtime
version when filing a bug; avoid attaching personal scans without redacting them.
