#!/usr/bin/env bash
# Read-only link check for the mdBook sources in book/src.
# Usage: check-book-links.sh [page.md ...]   (paths relative to the repo root
#        or to book/src; default = every page listed in SUMMARY.md)
# FAIL: SUMMARY.md entry missing on disk; link target missing or not listed
#       in SUMMARY.md (mdBook would not render it, so the link 404s).
# WARN: #anchor matches no heading slug in the target page.
# Exit 1 if any FAIL.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
src="$root/book/src"
summary="$src/SUMMARY.md"
fails=0; warns=0
fail() { echo "FAIL $*"; fails=$((fails + 1)); }

listed=$(grep -o '([^)]*\.md)' "$summary" | tr -d '()' | sort -u)
while read -r p; do
  [ -f "$src/$p" ] || fail "SUMMARY.md: $p — listed file is missing"
done <<<"$listed"

slugs() { # heading slugs of a page, outside code fences
  awk '/^```/{c=!c} !c && /^#+ /' "$1" | sed -E 's/^#+ +//' | tr 'A-Z' 'a-z' |
    sed -E 's/[^a-z0-9 _-]//g; s/ /-/g'
}

if [ $# -gt 0 ]; then pages=(); for a in "$@"; do
  if [ -f "$root/$a" ]; then pages+=("$(realpath "$root/$a")"); else pages+=("$(realpath -m "$src/$a")"); fi
done; else pages=(); while read -r p; do pages+=("$src/$p"); done <<<"$listed"; fi

for page in "${pages[@]}"; do
  [ -f "$page" ] || { fail "$page — page not found"; continue; }
  rel=${page#"$root"/}
  while IFS=$'\t' read -r ln target; do
    case "$target" in http://*|https://*|mailto:*|'#'*|'') continue ;; esac
    file=${target%%#*}; anchor=""; [[ "$target" == *'#'* ]] && anchor=${target#*#}
    [ -n "$file" ] || continue
    abs=$(realpath -m "$(dirname "$page")/$file")
    if [ ! -f "$abs" ]; then fail "$rel:$ln: $target — file does not exist"; continue; fi
    if [[ "$file" == *.md ]]; then
      if ! grep -qxF "${abs#"$src"/}" <<<"$listed"; then
        fail "$rel:$ln: $target — exists but is not listed in SUMMARY.md (404 when rendered)"; continue
      fi
    fi
    if [ -n "$anchor" ] && [[ "$file" == *.md ]] && ! slugs "$abs" | grep -qxF "$anchor"; then
      echo "WARN $rel:$ln: $target — no heading slug '$anchor'"; warns=$((warns + 1))
    fi
  done < <(awk '/^```/{c=!c} !c{s=$0; while (match(s, /\]\([^)]*\)/)) {t=substr(s, RSTART+2, RLENGTH-3); sub(/[ \t].*/, "", t); print NR "\t" t; s=substr(s, RSTART+RLENGTH)}}' "$page")
done
echo "checked ${#pages[@]} page(s): $fails FAIL, $warns WARN"
[ "$fails" -eq 0 ]
