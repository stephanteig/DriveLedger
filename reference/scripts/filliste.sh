#!/bin/zsh
# Lists every file in a folder/drive to a TSV on the Desktop. Read-only on the target.
# Usage: zsh ~/filliste.sh "/Volumes/DriveName/optional/subfolder"

DRIVE="${1:?Usage: zsh ~/filliste.sh \"/path/to/drive-or-folder\"}"
OUT=~/Desktop/"$(basename "$DRIVE")-filliste.tsv"
TAB=$'\t'

cd "$DRIVE" || { echo "Cannot open: $DRIVE"; exit 1; }

{
  printf "size_mb\tmodified\tpath\n"
  find . \( -name '.Spotlight-V100' -o -name '.fseventsd' -o -name '.Trashes' \
            -o -name '.TemporaryItems' -o -name '.DocumentRevisions-V100' \) -prune \
       -o -type f ! -name '.DS_Store' ! -name '._*' \
       -exec stat -f "%z${TAB}%Sm${TAB}%N" -t '%Y-%m-%d' {} + \
  | awk -F'\t' -v OFS='\t' '{$1=sprintf("%.1f",$1/1048576); sub(/^\.\//,"",$3); print}' \
  | sort -t"$TAB" -k3
} > "$OUT"

echo "Saved $(( $(wc -l < "$OUT") - 1 )) files to $OUT"
