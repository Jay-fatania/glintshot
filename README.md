# Glintshot

Cross-platform screenshot and screen-recording app (Tauri 2 + Rust).
This is the milestone 0 spike: region screenshots and region recording on Linux X11, plus benchmarks.
See [docs/SPEC.md](docs/SPEC.md) for the full plan and [docs/spike-results.md](docs/spike-results.md) for measurements.

## Setup (Ubuntu 22.04)

```sh
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev \
  libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config libx11-dev \
  libxrandr-dev libxcb1-dev libdbus-1-dev libpipewire-0.3-dev libclang-dev ffmpeg
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
npm install
```

## Run

```sh
npm run tauri dev                      # app with hot reload
cd src-tauri && cargo run --release --bin capture_bench 10   # capture benchmark
scripts/record_bench.sh 15            # recording/encoder benchmark (covers primary monitor)
cd src-tauri && cargo test                              # unit tests
cd src-tauri && cargo test --test record_smoke -- --ignored   # recorder test (needs X11)
cd src-tauri && cargo run --bin window_list             # windows the overlay can pick
```

By default screenshots go to `~/Pictures/Glintshot/` and recordings (MP4) to `~/Videos/Glintshot/`. Folders Glintshot creates are mode 700; files are 600.
After selecting a screenshot region, an editor toolbar offers arrow, rectangle, ellipse, text,
highlight, numbered markers, solid redaction and pixelate, with undo/redo, colours and sizes.
**Enter** saves the PNG and copies it; **Ctrl+C** copies only.

In the capture overlay: drag an area, click a window, or press **F** for the full screen.

Glintshot runs in the tray (closing the window hides it). Default global hotkeys on X11:
**Ctrl+Alt+S** screenshot, **Ctrl+Alt+R** record / stop recording.

The **Settings** tab sets save folders, hotkeys, what Enter does in the editor, recording frame rate
(30/60 fps) and start at login. Settings live in `~/.config/app.glintshot.spike/settings.json`.
`cd src-tauri && cargo run --bin hotkey_probe` reports which hotkeys are free or held by another app.

Recording uses FFmpeg with Intel/AMD GPU encoding (VA-API) and falls back to libx264; set `GLINTSHOT_ENCODER=x264` to force the fallback.

## Status

| Platform | Screenshot / recording | Notes |
|---|---|---|
| Ubuntu X11 | Working | Direct X11 `GetImage` via x11rb |
| Ubuntu Wayland | Not yet | Needs the ScreenCast portal; Rust PipeWire bindings don't build against Ubuntu 22.04's PipeWire 0.3.48 |
| Windows | Untested | xcap backend written, not compiled yet |
