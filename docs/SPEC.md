# Glintshot: Technical Spec (draft v0.1)

Name: Glintshot. Domain: glintshot.app (glintshot.com is registered by another owner; avoid using it). Targets: Ubuntu (Linux), Windows, iOS (companion).
Status: spec only. No code yet.

## 1. Goals

- Screenshot (region, window, full screen) with annotation, redaction, copy and save.
- Screen recording (region, window, full screen) with pause/resume, audio, click highlights, MP4 output.
- Smooth performance: capture and preview must not drop frames on mid-range hardware.
- Modern, fast UI that matches the platform look.
- Privacy and security by default: no silent updates, no telemetry unless opted in, private save folders.
- Closed-source friendly licensing.

## 2. Stack decision

**Chosen: Tauri 2 (Rust core + web UI), native capture in Rust.**

| Option | Verdict | Why |
|---|---|---|
| **Tauri 2 + Rust** | **Chosen** | One codebase for Linux and Windows (and macOS later). Small binaries, low idle RAM. Rust capture libraries use native OS APIs. Modern UI with web tooling. MIT/Apache-2.0. |
| Qt 6 + C++ | Runner-up | Best native performance and mature overlays. Needs C++ skills, and UI looks dated unless we work hard on it. Commercial licence needed unless LGPL rules are followed. |
| Slint + Rust | Watch | Pure-Rust native UI, very light. Desktop production readiness is less proven. Overlay and tray support is thinner. |
| Electron | Rejected | Heavy memory use. Capture is harder to control. |
| Flutter | Rejected | Needs platform plugins for all system-level capture and overlays. |

**Key risk:** Tauri overlays and transparent windows behave differently across platforms. Windows shows borders and shadows on borderless transparent windows. Linux depends on the compositor. A spike (section 9) tests this before we commit.

**Recheck before committing:** search results were thin for 2026 specifics. Verify current versions and open issues for Tauri, `windows-capture`, `xcap`/`scap` and `ashpd`.

## 3. Architecture

```
┌───────────────────────── Rust core (shared crate) ─────────────────────────┐
│ capture/   trait ScreenSource { list_displays, list_windows, grab_frame,   │
│            start_stream, stop }                                            │
│ encode/    H.264/HEVC via hardware encoders (see section 5), MP4 muxing     │
│ annotate/  shape model + renderer (undo/redo), redaction (solid, pixelate) │
│ ocr/       text extraction (phase 3)                                       │
│ store/     save paths, settings, capture history (SQLite)                   │
│ share/     upload + expiring links (phase 4)                                │
│ ipc/       commands and events to the UI                                    │
└───────────────────────────────────────────────────────────────────────────┘
        │ Tauri commands/events           │ FFI (UniFFI) for iOS (phase 5)
┌───────┴─────────────────────┐     ┌─────┴─────────────────────────┐
│ UI: Tauri webview (React or │     │ iOS: SwiftUI app + ReplayKit  │
│ Svelte + TypeScript)        │     │ broadcast extension, shares   │
│ overlays, toolbar, editor,  │     │ the Rust core via UniFFI      │
│ settings                    │     │                               │
└─────────────────────────────┘     └───────────────────────────────┘

Platform backends behind the `ScreenSource` trait:
- Linux X11:      xcap (screenshots) + FFmpeg x11grab or XCB capture
- Linux Wayland:  xdg-desktop-portal ScreenCast via `ashpd` + PipeWire (DMA-BUF)
- Windows:        windows-capture (Windows.Graphics.Capture); DXGI duplication as fallback
- iOS:            ReplayKit (no system-wide capture is allowed on iOS)
```

## 4. Platform capabilities

| Capability | Ubuntu X11 | Ubuntu Wayland | Windows 10/11 | iOS |
|---|---|---|---|---|
| Screenshot | Yes | Via portal (picker each time unless restore token) | Yes | Photos import only |
| Global hotkey | Yes (XGrabKey) | Limited, via desktop shortcut or portal shortcuts | Yes | N/A |
| Window capture | Yes | Via portal picker | Yes | N/A |
| Region overlay | Yes | Yes (compositor-dependent) | Yes (watch borders and shadows) | N/A |
| Recording | Yes | Yes (PipeWire) | Yes | ReplayKit broadcast only |
| System audio | PipeWire / Pulse monitor | PipeWire | WASAPI loopback | ReplayKit audio |

## 5. Video pipeline

- **Encoders:** hardware H.264 first (NVENC, AMF/QSV via FFmpeg, VAAPI on Linux), software x264 as fallback.
- **Licensing:** build FFmpeg as LGPL, or use hardware encoders only. Building with x264 (GPL) makes the whole app GPL. Confirm before we ship.
- **Container:** MP4 by default, GIF optional, WebM optional.
- **Segments:** pause and resume write segments; the final file is merged by remuxing (no re-encode).
- **Preview:** small, low-frame-rate thumbnail in the UI; full-rate capture stays in Rust.

## 6. Security and privacy requirements

1. No silent or automatic updates. Updates are shown and installed only when the user agrees.
2. No mandatory-update blocking. The app must keep working offline.
3. Update check is off by default and can be turned off.
4. Telemetry is off by default and opt-in.
5. Capture folders are created with user-only permissions (700 on Linux; user-only ACL on Windows).
6. Redaction: solid black and pixelate only. Blur is not offered as a way to hide secrets.
7. Uploads (phase 4) are opt-in, use TLS, and support link expiry.
8. Code signing on all platforms; signed Windows installer (MSIX or signed MSI); Flatpak or signed `.deb` on Linux.
9. Dependency audit in CI (`cargo audit`, `npm audit`) and SBOM for each release.
10. Permission prompts on every platform clearly say what is being recorded.

## 7. Feature scope by phase

- **Phase 1 (Desktop MVP):** region, window and full-screen screenshot; annotation (arrow, rectangle, ellipse, text, highlight, numbered markers, solid redact, pixelate); copy, save, discard; tray; global hotkeys; settings.
- **Phase 2 (Recording):** region and window recording; MP4; pause/resume; system audio and microphone; click highlights; timer badge; recording border; trim.
- **Phase 3 (Differentiators):** OCR; automatic PII redaction (emails, keys, card numbers); scrolling capture; pinned screenshots; capture history; command-line interface.
- **Phase 4 (Sharing):** upload to company storage or S3; expiring links; Slack and Jira share; company SSO sign-in.
- **Phase 5 (iOS):** SwiftUI app; ReplayKit recording; annotation of imported images; sync with desktop.
- **Later:** macOS port; webcam bubble; zoom-follows-cursor editing.

## 8. Team and tooling with Claude

- Claude Code writes the Rust core, platform backends and UI; humans review every security-sensitive change (capture permissions, updater, uploads, encryption).
- Repos: one monorepo (`core/` Rust crate, `desktop/` Tauri app, `ios/` Swift app, `docs/`).
- CI: GitHub Actions matrix for Ubuntu, Windows and macOS (for tests); signing on tagged releases.
- Each phase ends with a demo build and a written test plan.

## 9. Milestone 0: two-week spike (go or no-go)

Build a throwaway prototype to prove the risky parts before committing:

1. Tauri 2 transparent full-screen overlay for region selection on Ubuntu (X11 and Wayland) and Windows 11. Pass if the overlay has no visible border or shadow and stays responsive.
2. Capture 1080p at 60 fps from the Rust side on each OS. Measure CPU and dropped frames for 5 minutes.
3. Hardware H.264 encode via FFmpeg on an NVIDIA or Intel GPU. Measure CPU use.
4. Portal screenshot and recording on Ubuntu Wayland with a saved restore token.
5. Report: frame drops, CPU and RAM numbers, blockers. If the overlay fails, switch to Qt 6 for the UI and keep the Rust core.

## 10. Open questions

1. ~~App name and brand.~~ Resolved: Glintshot, domain glintshot.app. Still to do: USPTO and EUIPO clearance by a trademark attorney; register glintshot.app, plus social handles.
2. Who owns the code: company or personal? (Affects whether we reuse any ScreenFlame ideas. We are not reusing its code.)
3. Is iOS a must for version 1, or can it follow after desktop?
4. Company storage target for sharing (S3, SharePoint, Google Drive, or none in v1).
5. Hardware to test on: which Ubuntu version and GPU, which Windows machines.
6. Target release date for an internal beta.

## 11. Next steps

1. Answer the open questions above.
2. Confirm the Tauri choice after the milestone 0 spike.
3. Create the repository and CLAUDE.md with coding rules, the phase 1 task list and the security rules from section 6.
