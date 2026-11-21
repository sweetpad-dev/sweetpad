#!/usr/bin/env bash
# Renders every diagrams/<group>/<name>.d2 to
# static/images/blog/<group>/<name>-{light,dark}.svg. Needs d2 on PATH.
set -euo pipefail
cd "$(dirname "$0")"

for source in */*.d2; do
  group="$(dirname "$source")"
  name="$(basename "$source" .d2)"
  out="../static/images/blog/$group"
  mkdir -p "$out"
  for theme in light dark; do
    cat "theme-$theme.d2" classes.d2 "$source" |
      d2 --layout elk --elk-nodeNodeBetweenLayers 36 --sketch --pad 10 --scale 0.6 - "$out/$name-$theme.svg"
  done
done
