# Native accessibility

NoteGen enables eframe 0.37's AccessKit integration in desktop builds. On Windows,
the locked AccessKit Windows adapter exposes the egui widget tree through UI
Automation. Both the WGPU and OpenGL eframe integrations handle initial tree
requests and accessibility actions. This is partial native support: a completed
Windows Narrator audit is **not yet recorded**.

Named controls, keyboard focus, and announcements must be tested with assistive
technology in the packaged application. Headless widget-tree tests cannot prove
what Narrator says, whether a native dialog is usable, or whether focus remains
visible at every display scale.

## Automated checks

```powershell
cargo metadata --locked --all-features --format-version 1 > metadata.json
cargo test --locked --features desktop --lib accessibility_tests
```

The Experience tests inspect real `egui::PlatformOutput::accesskit_update` trees.
They cover named OCR/search inputs, one-time Settings tab focus, stable polite
atomic task results without focus changes, named source-page images and region
descriptions, and generated drafts with live announcements disabled. Draft text
remains readable by explicitly navigating to it; it is not announced after each
reader token. These tests do not require a window or screen reader.

Dependency resolution was checked locally with `--locked --offline --all-features`.
Enabling the bridge added 17 locked adapter/transitive packages and changed no
previously resolved package versions. Compilation and runtime test results belong
in the release test report; dependency resolution alone is not a passing test.

## Windows Narrator audit — pending

Use a release build on a normal Windows desktop. Record Windows version, NoteGen
commit, renderer, display scaling, Narrator version/settings, input method, and
the result for every step. Repeat with the other renderer and at least 100% and
200% display scaling. Use a disposable source with known text and a table.

1. Start Narrator before opening NoteGen. Navigate the main window using Tab and
   Shift+Tab. Confirm each focused control has a visible focus indicator and an
   announced name, role, state and value. Confirm disabled controls are identified.
2. Import a source with Ctrl+O and use the native file dialog entirely by keyboard.
   Inspect the output-folder control, page range, preset, queue priority and queue
   move/remove actions. Confirm duplicate row actions identify the relevant input.
3. Open Settings. Confirm focus enters the selected tab once, then stays where the
   user moves it. Change tabs, OCR languages, scale and reduced motion. Search the
   configuration reference, clear the search, close Settings, and verify focus
   returns to a useful main-window control.
4. Open the command palette with Ctrl+K. Confirm its search field receives focus,
   results are named, Enter activates a result, and closing it does not trap focus.
5. Run one-page conversion. Confirm discrete start, completion, cancellation and
   failure results can be heard and revisited. Confirm progress or streamed draft
   changes do not repeatedly interrupt typing or reader navigation. Confirm the
   live source image and highlighted-region description can be found.
6. Open review. Navigate pages and blocks by keyboard. Confirm table cell editors
   announce page, block, row and column; text and figure editors identify their
   purpose; approval, save, undo and redo expose their state and outcome. Ensure
   source zoom and text are usable without interpreting color alone.
7. Exercise project search, replacement preview/exclusions, Word import selection,
   resource controls, credential entry and export. Confirm password fields do not
   expose the entered token in their accessible value or in announcements. Inspect
   result/error messages and open/save dialogs without a mouse.
8. Close and reopen tool windows and the application. Confirm there are no keyboard
   traps, stale focus, duplicate live announcements or unlabeled interactive nodes.

Store observed failures with exact reproduction steps. Do not mark the audit
complete from the existence of AccessKit nodes or from visual screenshots alone.

## Implementation references

- [eframe native integrations](https://github.com/emilk/egui/tree/main/crates/eframe/src/native)
- [egui accessibility support](https://github.com/emilk/egui/tree/main/crates/egui/src)
- [AccessKit Windows adapter](https://github.com/AccessKit/accesskit/tree/main/platforms/windows)

The implementation was checked against the locally resolved eframe/egui 0.37.0,
accesskit_winit 0.34.2 and accesskit_windows 0.35.2 source, rather than assuming
that upstream defaults were enabled in NoteGen.
