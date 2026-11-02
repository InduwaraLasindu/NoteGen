# NoteGen 0.4.1

This patch fixes the live AI view introduced in 0.4.0.

- Context-assisted re-reading announces the actual source page and active reader before inference.
- Draft updates are accepted only for the displayed source page. Clearing a view prevents queued draft text from bringing it back until a fresh source is announced.
- Clearing or disabling previews releases the source texture. Reader and crop changes reset the draft while retaining the reusable source image.
- Temporary hardware-test previews are cleared on completion, cancellation and error exits before temporary files are removed.
- Preview image sizes respect the graphics adapter's texture limit.

The release retains the 0.4 one-click installer AI setup, verified model/runtime updates, dependency servicing through app updates, measured hardware test, master progress estimate, writer memory, clarification, review and Word/font export.

Validation gates cover 109 ordinary tests on Linux and Windows, strict format/Clippy, native raster-PDF OCR, a loopback-reader regression that reproduces the released bug, confirmed-text preservation, stale/disabled/legacy preview events, interrupted hardware cleanup, actual CPU model inference and Maximum conversion, and installation/upgrade from 0.4.0 with AI setup, cancellation and data retention. CI smoke images use a 768-pixel cap; production quality settings are unchanged. GPU performance and broad natural handwriting still require separate acceptance on representative hardware and scans.

Install the new EXE over 0.4.0. Models, writer profiles and document projects are retained. The installer is unsigned; checksums and provenance accompany the release.
