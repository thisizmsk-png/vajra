#!/usr/bin/env bash
# Renders every shipped icon from the SVG sources in src/.
# Needs rsvg-convert (librsvg) and, for icon.icns, macOS iconutil.
# Usage: app/core/icons/build-icons.sh
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
src="$here/src"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

render() { # render <svg> <pixels> <out.png>
  rsvg-convert -w "$2" -h "$2" -o "$3" "$src/$1"
}

# The 16 and 32 px drawings are separate, pixel-aligned sources. Scaling the
# master down to those sizes closes the gaps between the prongs.
source_for() {
  case "$1" in
    16) echo app-icon-16.svg ;;
    32) echo app-icon-32.svg ;;
    *) echo app-icon.svg ;;
  esac
}

# App icon: the macOS iconset, then the PNGs tauri.conf.json lists.
set_dir="$work/icon.iconset"
mkdir "$set_dir"
for pt in 16 32 128 256 512; do
  render "$(source_for "$pt")" "$pt" "$set_dir/icon_${pt}x${pt}.png"
  px=$((pt * 2))
  render "$(source_for "$px")" "$px" "$set_dir/icon_${pt}x${pt}@2x.png"
done
iconutil -c icns -o "$here/icon.icns" "$set_dir"

cp "$set_dir/icon_32x32.png" "$here/32x32.png"
cp "$set_dir/icon_128x128.png" "$here/128x128.png"
cp "$set_dir/icon_128x128@2x.png" "$here/128x128@2x.png"
cp "$set_dir/icon_512x512.png" "$here/icon.png"

# Menu-bar template marks: black plus alpha, recoloured by macOS.
# 16 and 18 px have their own pixel-aligned drawings.
mkdir -p "$here/menubar"
mark_for() {
  case "$1" in
    16) echo menubar-mark-16.svg ;;
    18) echo menubar-mark-18.svg ;;
    *) echo menubar-mark.svg ;;
  esac
}
for pt in 16 18 32; do
  render "$(mark_for "$pt")" "$pt" "$here/menubar/markTemplate-${pt}.png"
  px=$((pt * 2))
  render "$(mark_for "$px")" "$px" "$here/menubar/markTemplate-${pt}@2x.png"
done

echo "icons written to $here"
