#!/usr/bin/env bash
# Recording spike: compares FFmpeg x11grab + encoder pipelines on X11.
#
# For each encoder it records the primary monitor for $SECONDS_PER_RUN seconds
# while a moving test pattern plays full-screen on it (static desktops are too
# easy to encode), then reports delivered frames, CPU use and file size.
#
# Usage: scripts/record_bench.sh [seconds_per_run] [encoders...]
#   encoders: x264 vaapi qsv (default: all three)
set -euo pipefail

SECONDS_PER_RUN="${1:-15}"
shift || true
if (($#)); then ENCODERS=("$@"); else ENCODERS=(x264 vaapi qsv); fi

FPS=60
OUT_DIR="${OUT_DIR:-$(mktemp -d)}"
mkdir -p "$OUT_DIR"
NPROC="$(nproc)"

# Primary monitor geometry from xrandr, e.g. "1920x1200+0+0".
geometry="$(xrandr --query | awk '/ connected primary/ {for (i=1;i<=NF;i++) if ($i ~ /^[0-9]+x[0-9]+\+/) {print $i; exit}}')"
[[ -n "$geometry" ]] || { echo "could not find primary monitor" >&2; exit 1; }
size="${geometry%%+*}"
offset="${geometry#*+}"; off_x="${offset%%+*}"; off_y="${offset#*+}"
width="${size%x*}"; height="${size#*x}"

echo "display $DISPLAY, primary monitor ${size} at ${off_x},${off_y}, ${FPS} fps, ${SECONDS_PER_RUN}s per run"
echo "output: $OUT_DIR"
echo

encoder_args() {
  case "$1" in
    x264)  echo "-vf format=yuv420p -c:v libx264 -preset ultrafast -crf 23" ;;
    vaapi) echo "-vf format=nv12,hwupload -c:v h264_vaapi -qp 23" ;;
    qsv)   echo "-init_hw_device qsv=hw -filter_hw_device hw -vf format=nv12,hwupload=extra_hw_frames=64 -c:v h264_qsv -global_quality 23" ;;
    *) echo "unknown encoder $1" >&2; exit 1 ;;
  esac
}

printf "%-6s %8s %8s %8s %8s %9s %s\n" encoder frames expect fps cpu% size_MB note
for enc in "${ENCODERS[@]}"; do
  out="$OUT_DIR/$enc.mp4"
  log="$OUT_DIR/$enc.log"

  # Moving content: test pattern with a running counter, on top of the monitor.
  ffplay -loglevel quiet -noborder -alwaysontop -left "$off_x" -top "$off_y" -x "$width" -y "$height" \
    -f lavfi -i "testsrc2=size=${size}:rate=${FPS}:duration=$((SECONDS_PER_RUN + 4))" -autoexit &
  player=$!
  sleep 1.5

  # shellcheck disable=SC2046
  if /usr/bin/time -f '%e %U %S' -o "$OUT_DIR/$enc.time" \
       ffmpeg -hide_banner -y -nostdin \
         $( [[ $enc == vaapi ]] && echo "-vaapi_device /dev/dri/renderD128" ) \
         -f x11grab -framerate "$FPS" -video_size "$size" -i "${DISPLAY}+${off_x},${off_y}" \
         -t "$SECONDS_PER_RUN" \
         $(encoder_args "$enc") \
         "$out" > "$log" 2>&1; then
    note=""
  else
    note="FAILED (see $log)"
  fi
  wait "$player" 2>/dev/null || true

  if [[ -z "$note" ]]; then
    frames="$(ffprobe -v error -count_frames -select_streams v:0 -show_entries stream=nb_read_frames -of csv=p=0 "$out")"
    read -r elapsed user sys < "$OUT_DIR/$enc.time"
    cpu="$(awk -v u="$user" -v s="$sys" -v e="$elapsed" -v n="$NPROC" 'BEGIN {printf "%.1f", (u+s)/e*100/n}')"
    fps="$(awk -v f="$frames" -v t="$SECONDS_PER_RUN" 'BEGIN {printf "%.1f", f/t}')"
    mb="$(awk -v b="$(stat -c %s "$out")" 'BEGIN {printf "%.1f", b/1048576}')"
    dups="$(grep -o 'dup=[0-9]*' "$log" | tail -1 || true)"
    drops="$(grep -o 'drop=[0-9]*' "$log" | tail -1 || true)"
    note="${dups} ${drops}"
    printf "%-6s %8s %8s %8s %8s %9s %s\n" "$enc" "$frames" "$((FPS * SECONDS_PER_RUN))" "$fps" "$cpu" "$mb" "$note"
  else
    printf "%-6s %s\n" "$enc" "$note"
  fi
done
echo
echo "cpu% is the share of all ${NPROC} CPU threads used by ffmpeg (100% = every thread busy)."
