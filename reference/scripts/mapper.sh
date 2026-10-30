#!/bin/zsh
# Folder overview (size, file count, path) to a TSV on the Desktop. Read-only on the target.
# Usage: zsh ~/mapper.sh "/Volumes/DriveName/optional/subfolder" [depth, default 4]

DRIVE="${1:?Usage: zsh ~/mapper.sh \"/path/to/drive-or-folder\" [depth]}"
DEPTH="${2:-4}"
OUT=~/Desktop/"$(basename "$DRIVE")-mapper.tsv"

cd "$DRIVE" || { echo "Cannot open: $DRIVE"; exit 1; }

{
  printf "size\tfiles\tpath\n"
  find . -mindepth 1 -maxdepth "$DEPTH" -type d ! -path './.*' | sort | while IFS= read -r d; do
    printf "%s\t%s files\t%s\n" \
      "$(du -sh "$d" 2>/dev/null | cut -f1)" \
      "$(find "$d" -type f ! -name '.*' | wc -l | tr -d ' ')" \
      "${d#./}"
  done
} > "$OUT"

echo "Saved $(( $(wc -l < "$OUT") - 1 )) folders to $OUT"
