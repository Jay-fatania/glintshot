# Phase 1 test plan: desktop MVP

Scope comes from [SPEC.md](SPEC.md) section 7 (Phase 1) and the security rules in section 6.
Phase 1 is done when every P0 case passes on Ubuntu X11 and Windows, and no open defect is Critical or High.

## 1. Scope

In scope:
- Screenshot capture: region, window, full screen.
- Annotation editor: arrow, rectangle, ellipse, highlight, solid redact, pixelate, text, numbered markers, undo/redo, colour and size.
- Output: save (PNG to the screenshot folder), copy to clipboard, save and copy, discard.
- Tray icon and menu; close hides the window to the tray.
- Global hotkeys (default Ctrl+Alt+S screenshot, Ctrl+Alt+R record/stop).
- Settings tab: folders, hotkeys, Enter action, recording frame rate, start at login.
- Privacy and security requirements that apply in Phase 1 (section 6, items 1–6, 10).

Out of scope for Phase 1 (covered by later phases):
- Phase 2 recording features (pause/resume, audio, click highlights, timer badge, trim) and window recording.
- OCR, PII redaction, scrolling capture, history, CLI.
- Uploads and sharing, iOS, Wayland.

Phase 1 has no network features, so no test in this plan should need network access. Section 6 item 3 (update check off by default) is verified by absence: there is no updater code.

## 2. Test environments

| ID | Platform | Notes |
|---|---|---|
| E1 | Ubuntu 22.04.5, X11, Intel Alder Lake-P iGPU | Primary. Two monitors (1920×1200 primary, 1920×1080 secondary). Used for the spike. |
| E2 | Windows 10 or 11 | Needs a signed or unsigned NSIS build from CI. Hardware to be confirmed (SPEC section 10, item 5). |
| E3 | Ubuntu X11 without VA-API (or with `GLINTSHOT_ENCODER=x264`) | Software encoder fallback. |
| E4 | Ubuntu X11, single monitor, 150% scaling if available | HiDPI and single-monitor coordinate checks. |

Wayland is out of scope for Phase 1 (see `docs/spike-results.md`, finding 1).

Each run records: build commit SHA, OS version, GPU, number of monitors, and display scaling.

## 3. Test types

- **Automated (A):** run in CI (`.github/workflows/build.yml`) or by `cargo test`. Covers settings logic, parsing, and encoder/recorder smoke tests.
- **Manual (M):** run by a person on the environments above. Covers the overlay, the editor, tray behaviour, hotkeys, and permissions, which need real display and input.
- **Verification by inspection (I):** check a file, permission, or the absence of code.

Automated coverage that exists today:
- `src-tauri/src/settings.rs` unit tests: defaults, missing file, partial file, corrupt file, save/load round trip, rejected settings.
- `src-tauri/tests/record_smoke.rs` (ignored by default, needs X11): `cargo test --test record_smoke -- --ignored`.
- CI: `tsc --noEmit`, `cargo clippy -D warnings`, `cargo test`.

Gaps: there are no automated tests for the frontend (overlay, editor, settings UI), the annotation geometry, or the tray and hotkeys. Those are manual until a test harness is added (see section 7).

## 4. Priorities

- **P0:** a core flow or security requirement. A failure blocks the phase.
- **P1:** a feature in scope. A failure needs a fix or a documented workaround before the phase closes.
- **P2:** polish or edge cases. A failure can be logged for later.

## 5. Test cases

### 5.1 Capture overlay

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| CAP-01 | M | P0 | Trigger screenshot hotkey. Drag an area on the primary monitor. | Overlay shows the frozen screen. The selection is saved and the saved PNG matches the selected area pixel for pixel. |
| CAP-02 | M | P0 | As CAP-01, on the secondary monitor. | Correct monitor image and the correct area. |
| CAP-03 | M | P0 | Hover a window. Click it. | The window under the cursor is highlighted with its title. The clicked window is captured, including its title bar. |
| CAP-04 | M | P0 | Press F. | The whole monitor under the cursor is captured. |
| CAP-05 | M | P0 | Press Esc during selection. | Overlay closes. Nothing is saved. |
| CAP-06 | M | P1 | Drag a very small area (a few pixels), and a zero-size click. | No crash. Either nothing is saved or a clear error is shown. |
| CAP-07 | M | P1 | Start a capture across a monitor boundary. | Selection is clipped to the monitor it started on, or handled as documented. No crash. |
| CAP-08 | M | P1 | Window list: hidden, minimised, docked and other-workspace windows. | They are not offered as capture targets. |
| CAP-09 | M | P1 | Repeat CAP-01 on E4 with HiDPI scaling. | Saved image has correct coordinates and is not offset or blurred. |
| CAP-10 | M | P2 | Capture while a fullscreen video or game is running. | Overlay appears. Capture is either correct or the limitation is documented. |

### 5.2 Annotation editor

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| ED-01 | M | P0 | Draw one of each tool: arrow, rectangle, ellipse, highlight, text, numbered marker. | Each shape appears where drawn, in the chosen colour and size. Saved file includes them. |
| ED-02 | M | P0 | Draw a solid redaction over text. Save. Open the PNG. | The redacted area is fully black. The underlying text cannot be read in the saved file. |
| ED-03 | M | P0 | Draw a pixelate region over text. Save. Open the PNG. | Text is not readable from the saved file. No blur tool is offered (SPEC section 6 item 6). |
| ED-04 | M | P0 | Undo and redo across mixed shapes, then edit more. | Undo and redo restore the exact prior state. No shapes disappear or duplicate. |
| ED-05 | M | P1 | Numbered markers: place 3, delete the middle one, place another. | Expected: numbers stay unique. `nextMarkerNumber` now returns the highest existing number + 1 (`src/annotations.ts`), so the new marker is numbered 4. Regression check for the fix. |
| ED-06 | M | P1 | Change colour and size, then draw. | New shapes use the new settings. Existing shapes are not changed unless selected. |
| ED-07 | M | P1 | Pixelate near the image edge and at the image corner. | No crash and no pixels from outside the image. |
| ED-08 | M | P2 | Text tool: empty text, very long text, Unicode text (e.g. emoji, accented letters). | Text renders. Empty text is not saved as an empty shape. |

### 5.3 Output: save, copy, discard

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| OUT-01 | M | P0 | Press Enter (default action: save). | PNG is written to the screenshot folder and the image is on the clipboard. |
| OUT-02 | M | P0 | Press Ctrl+C. | Image is on the clipboard. No file is written. |
| OUT-03 | M | P0 | Press Ctrl+S. | File written and copied. |
| OUT-04 | M | P0 | Discard (close the editor without saving). | No file is written. Clipboard is unchanged. |
| OUT-05 | I | P0 | Check permissions of a folder created by Glintshot and of a saved screenshot. | Folder is mode 700 (`ensure_dir`). Screenshot is mode 600 (`write_private_png`). Check with `ls -ld` and `ls -l`. |
| OUT-06 | I | P0 | Pick an existing folder (e.g. `~/Desktop`) as the save folder. | Its permissions do not change (settings.rs `ensure_dir` only applies to folders Glintshot creates). |
| OUT-07 | M | P1 | Save to a folder that is read-only. | Error is shown in the overlay and the clipboard is not changed (`finish_screenshot` returns before copying). Overlay stays open. |
| OUT-07b | M | P1 | Save succeeds but the clipboard fails (e.g. no clipboard manager on a headless session). | Expected: the PNG is kept, the overlay closes, and a `capture-error` says "Saved to <path>, but the clipboard copy failed". The result has `copied: false`, so the main window does not show a Clipboard row. Fixed in `finish_screenshot`. |
| OUT-08 | M | P1 | Enter action set to "copy" in Settings. Press Enter. | Behaves as Ctrl+C. Ctrl+S still saves. |
| OUT-09 | M | P1 | Two saves in the same millisecond. | Expected: distinct files. `unique_png_path` adds a `-2`, `-3`, ... suffix when the name is taken (unit test `unique_png_path_avoids_existing_files`). Also check two saves back to back by script. |

### 5.4 Tray and window lifecycle

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| TRY-01 | M | P0 | Click the window close button. | Window hides. Tray icon stays. App keeps running. |
| TRY-02 | M | P0 | Tray menu: Capture, Record, Settings, Quit. | Each item does what its label says. Quit exits the process. |
| TRY-03 | M | P1 | Restore the window from the tray after hiding. | Window comes back with the correct tab and state. |
| TRY-04 | M | P1 | Tray icon on a desktop without a tray (GNOME without the AppIndicator extension). | App does not crash. Behaviour is documented. |

### 5.5 Global hotkeys

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| HK-01 | M | P0 | Ctrl+Alt+S from another app, with the Glintshot window hidden. | Capture overlay opens. |
| HK-02 | M | P0 | Ctrl+Alt+R starts a recording. Press it again. | Recording starts, then stops and saves to the video folder. |
| HK-03 | M | P1 | Run `cargo run --bin hotkey_probe` while another app holds the shortcut. | Probe reports the conflict. |
| HK-03b | M | P1 | Start the app while another app holds Ctrl+Alt+S. | Registration failure is recorded per hotkey (`register_hotkeys`, lib.rs ~502). Check that the failure is visible in Settings (not verified in code yet). The other hotkey still works. |
| HK-04 | M | P1 | Change a hotkey in Settings by pressing new keys. Try a conflicting one. | New hotkey works after save. Conflicts are reported. Hotkeys are paused while recording a new one. |
| HK-05 | M | P1 | Press a hotkey while the settings field is focused. | Hotkey is captured into the field, not triggered as a capture. |
| HK-06 | M | P2 | Restart the app. | Saved hotkeys are active. |

### 5.6 Settings

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| SET-01 | A | P0 | `cargo test` in `src-tauri`. | All settings unit tests pass. |
| SET-02 | M | P0 | Change each setting, Save, restart the app. | Values persist. The Save button is disabled when nothing has changed. |
| SET-03 | M | P0 | Edit `~/.config/app.glintshot.spike/settings.json` to invalid JSON. Start the app. | App starts with defaults. The invalid file is not crashed on. |
| SET-04 | M | P1 | Unknown keys in the settings file. | Ignored. Known keys still load. |
| SET-05 | M | P1 | Invalid values in the UI (e.g. a bad hotkey string, a folder that does not exist). | Validation message shown. Invalid value is not saved. |
| SET-06 | I | P0 | Check the settings file mode after save. | Mode 600. |
| SET-07 | M | P1 | Recording frame rate 30 and 60. | Output frame rate matches. |
| SET-08 | M | P1 | Start at login enabled, then disabled. | Autostart entry is created and removed. Login start passes `--background` (lib.rs `BACKGROUND_ARG`) and stays in the tray. |

### 5.7 Recording (Phase 1 subset)

Recording is in the repo already, but the full Phase 2 feature set is not part of Phase 1. These cases cover only the region recording that Phase 1 depends on for hotkeys and settings.

| ID | Type | Pri | Steps | Expected |
|---|---|---|---|---|
| REC-01 | A | P0 | `cargo test --test record_smoke -- --ignored` on E1. | Test passes. |
| REC-02 | M | P0 | Region recording at 60 fps, 15 s, on E1 with VA-API. | Playable MP4. Frame count and smoothness close to `docs/spike-results.md` finding 3. |
| REC-03 | M | P0 | Same on E3 (`GLINTSHOT_ENCODER=x264`). | Playable MP4. Fallback is used and reported. |
| REC-04 | M | P1 | Stop recording from the hotkey and from the tray. | Both stop the recording and produce a playable file. |

### 5.8 Privacy and security (SPEC section 6)

| ID | Type | Pri | Check | Expected |
|---|---|---|---|---|
| SEC-01 | I | P0 | Item 1: no automatic update check or install code. | No updater plugin in `Cargo.toml` or `tauri.conf.json`. |
| SEC-02 | I | P0 | Item 4: no telemetry. | No analytics or telemetry crates or network calls in `src` and `src-tauri/src`. |
| SEC-03 | M | P0 | Item 2: app works with no network. | All Phase 1 flows (CAP, ED, OUT, HK, SET) pass with networking disabled. |
| SEC-04 | I | P0 | Item 5: folder permissions (see OUT-05, OUT-06). | Verified there. |
| SEC-05 | I | P0 | Item 6: no blur redaction. | Only solid black and pixelate exist in `annotations.ts`. |
| SEC-06 | M | P1 | Item 10: the capture permission prompts on each platform say what is recorded. | Verified on E1 and E2. Wayland prompts are out of scope. |
| SEC-07 | I | P1 | Settings and captures are not written to world-readable locations. | Only the app config dir and the chosen folders are used. |

## 6. Exit criteria

- All P0 cases pass on E1 and E2 (E2 only for cases that apply on Windows; record which ones are skipped).
- No open Critical or High defect.
- P1 failures have a fix or a documented workaround.
- The CI workflow is green on `main`.
- A demo build (CI artifact for E1 and E2) is produced.

## 7. Gaps and next steps

1. Add frontend unit tests (Vitest) for the annotation geometry and the editor's undo/redo. The geometry in `src/annotations.ts` is pure and easy to test.
2. Add a Tauri integration test or a scripted E2E run (WebDriver) for the settings save/load flow.
3. Add an automated permissions check for OUT-05 and SET-06 in `record_smoke`-style Rust tests (Linux only).
4. Record manual test results in a table per run, with the commit SHA and environment from section 2.
