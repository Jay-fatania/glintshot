# Milestone 0 spike results

Test machine: Ubuntu 22.04.5, X11 (display `:1`), Intel i5-1240P (16 threads),
Intel Alder Lake-P integrated GPU, FFmpeg 4.4.2. Monitors: 1920×1200 laptop
panel (primary) + 1920×1080 HDMI.

## 1. Region overlay (Linux X11): pass

Freeze-frame approach: capture the monitor, show it in a full-screen,
undecorated, always-on-top Tauri window, drag to select. Confirmed working by
hand on 2026-10-14: right monitor, correct frozen image, saved PNG matches the
selection. No transparent window needed, so the Windows border and shadow
issues with transparent Tauri windows don't apply.

## 2. Screenshot capture speed: pass for screenshots

`capture_bench` (x11rb `GetImage`, both monitors, 3840×1200, release build, 10 s):

| mean | p50 | p95 | p99 | max | rate |
|---|---|---|---|---|---|
| 23.9 ms | 22.4 ms | 33.9 ms | 48.1 ms | 52.1 ms | 39 grabs/s |

Fine for screenshots (one grab). Too slow for 60 fps recording, so recording
uses FFmpeg `x11grab`, which uses X11 shared memory (MIT-SHM).

## 3. Recording with hardware encoding: pass (VA-API)

`scripts/record_bench.sh 15`: FFmpeg `x11grab` of the primary monitor
(1920×1200) at 60 fps for 15 s, with a moving full-screen test pattern
(`testsrc2`). CPU is ffmpeg's share of all 16 threads.

| Encoder | Frames (of 900) | CPU | Bitrate | Duplicated / dropped | Result |
|---|---|---|---|---|---|
| `h264_vaapi` (Intel GPU, QP 23) | 900 | 2.7% (≈0.4 core) | 19.5 Mbit/s | 1 / 0 | Smooth |
| `libx264` ultrafast (CPU, CRF 23) | 900 | 4.9% (≈0.8 core) | 23.9 Mbit/s | 26 / 24 | ~3% frames repeated or skipped |
| `h264_qsv` | – | – | – | – | Fails: Intel oneVPL runtime not installed; not needed since VA-API works |

The test pattern is much harder to encode than a normal desktop, so real
recordings will be far smaller. A frame from the VA-API file was checked by
eye: it shows the test pattern, GNOME top bar and dock.

## Findings that change the plan

1. **Wayland on Ubuntu 22.04:** the Rust PipeWire bindings (`pipewire` 0.8–0.10,
   used by `xcap` 0.5+) don't compile against PipeWire 0.3.48. Options: ship as
   a Flatpak (newer PipeWire in the runtime), require Ubuntu 24.04+ for Wayland,
   or record through GStreamer's `pipewiresrc` instead of the Rust bindings.
2. **Recorder design:** run FFmpeg as a separate process (`x11grab` on X11)
   with hardware encoders first (VA-API on Intel/AMD, NVENC on NVIDIA) and a
   software fallback. Proven here at 60 fps with almost no CPU.
3. **Licensing:** Ubuntu's FFmpeg includes libx264 (GPL). Calling the system
   `ffmpeg` as a separate program is fine, but any FFmpeg we bundle (Windows,
   Flatpak) must be an LGPL build, so the software fallback would be OpenH264
   instead of x264. To check in phase 2.
4. **X display:** this machine uses `:1`, not `:0`. Always read `$DISPLAY`.

## Not yet tested

- Windows (xcap backend written, not compiled).
- Wayland (blocked by finding 1).
- Recording both monitors at once, audio, pause/resume.
- NVIDIA NVENC (no NVIDIA GPU on this machine).
