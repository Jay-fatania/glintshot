# Glintshot

Cross-platform screenshot and screen-recording app (Tauri 2 + Rust).
This is the milestone 0 spike: region screenshots on Linux X11 plus a capture benchmark.
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
```

Screenshots are saved to `~/Pictures/Glintshot/` (folder mode 700, files 600).

## Status

| Platform | Screenshot | Notes |
|---|---|---|
| Ubuntu X11 | Working | Direct X11 `GetImage` via x11rb |
| Ubuntu Wayland | Not yet | Needs the ScreenCast portal; Rust PipeWire bindings don't build against Ubuntu 22.04's PipeWire 0.3.48 |
| Windows | Untested | xcap backend written, not compiled yet |
