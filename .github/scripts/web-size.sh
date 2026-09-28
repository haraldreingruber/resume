#!/usr/bin/env bash
# Web bundle sizes for the CI job summary: raw, gzip and brotli per asset,
# the brotli change from a baseline (the latest main build), and a budget
# check on the brotli size, which is what browsers download.
#
# Usage: web-size.sh <dist dir> <baseline file> <output file>
# Size files hold one line per asset: "<asset> <raw> <gzip> <brotli>". Without
# a baseline file, the change column stays empty.
set -euo pipefail

# Brotli budgets in bytes, about 15% above the sizes when they were set
# (September 2026: wasm 366,104, JS 13,441). Raise them deliberately.
declare -A BUDGET=([wasm]=420000 [js]=16000)

dist=$1
baseline=$2
out=$3
summary=${GITHUB_STEP_SUMMARY:-/dev/stdout}

: > "$out"
for asset in wasm js; do
  files=("$dist"/*."$asset")
  if [[ ${#files[@]} -ne 1 || ! -f ${files[0]} ]]; then
    echo "::error::expected exactly one .$asset file in $dist"
    exit 1
  fi
  file=${files[0]}
  echo "$asset $(wc -c < "$file") $(gzip -9c "$file" | wc -c) $(brotli -c "$file" | wc -c)" >> "$out"
done

# The baseline's brotli size of an asset (empty if unknown).
baseline_brotli() {
  if [[ -f $baseline ]]; then
    awk -v asset="$1" '$1 == asset { print $4 }' "$baseline"
  fi
}

# "+1234 (+0.3%)" from an old and a new size, or "n/a" without an old one.
change() {
  if [[ -z $1 ]]; then
    echo "n/a"
    return
  fi
  awk -v old="$1" -v new="$2" 'BEGIN { printf "%+d (%+.1f%%)", new - old, 100 * (new - old) / old }'
}

failed=0
{
  echo "### Web bundle"
  if [[ -f $baseline ]]; then
    echo "Changes are relative to the latest main build."
  else
    echo "No baseline from main yet (the next main build saves one)."
  fi
  echo
  echo "| Asset | Raw | gzip | brotli | brotli change | brotli budget |"
  echo "| --- | ---: | ---: | ---: | ---: | ---: |"
} >> "$summary"
while read -r asset raw gzip brotli; do
  budget=${BUDGET[$asset]}
  status="✅"
  if (( brotli > budget )); then
    status="❌ over"
    failed=1
    echo "::error::$asset is $brotli bytes brotli-compressed, over its budget of $budget (see .github/scripts/web-size.sh)"
  fi
  echo "| $asset | $raw | $gzip | $brotli | $(change "$(baseline_brotli "$asset")" "$brotli") | $budget $status |" >> "$summary"
done < "$out"
exit $failed
