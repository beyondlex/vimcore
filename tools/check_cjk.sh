#!/usr/bin/env bash
# Fail if a tracked (or untracked-but-not-ignored) file outside the .cjk-allow
# whitelist contains CJK characters.
#
#   tools/check_cjk.sh          # check; exit 1 and list offenders on violation
#   tools/check_cjk.sh --list   # print the parsed whitelist
#
# Runnable from any subdirectory. Needs git with PCRE support (`git grep -P`),
# which ships with stock macOS and Linux builds.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

ALLOW_FILE=.cjk-allow
CJK='[\x{4e00}-\x{9fff}]'

[ -f "$ALLOW_FILE" ] || { echo "error: $ALLOW_FILE not found" >&2; exit 1; }

tmp_allow=$(mktemp)
tmp_hits=$(mktemp)
trap 'rm -f "$tmp_allow" "$tmp_hits"' EXIT

# "path   # reason" -> "path"; drop blanks, comments and CR.
sed -e 's/#.*//' -e 's/[[:space:]]*$//' -e 's/\r$//' "$ALLOW_FILE" \
    | grep -v '^$' | sort -u > "$tmp_allow"

if [ "${1:-}" = "--list" ]; then
    cat "$tmp_allow"
    exit 0
fi

if [ ! -s "$tmp_allow" ]; then
    echo "error: $ALLOW_FILE has no entries -- refusing to run (an empty -f list matches everything)" >&2
    exit 1
fi

# Every file containing at least one CJK character. --untracked catches files
# that are about to be committed; gitignored trees (target/) stay out.
git grep -P -l --untracked "$CJK" -- . > "$tmp_hits" || true

violations=$(grep -vxF -f "$tmp_allow" "$tmp_hits" || true)

if [ -n "$violations" ]; then
    echo "CJK text found outside the whitelist:" >&2
    echo >&2
    while IFS= read -r f; do
        echo "  $f" >&2
        git grep -P -n --untracked "$CJK" -- "$f" 2>/dev/null | head -5 | sed 's/^/      /' >&2
    done <<< "$violations"
    echo >&2
    echo "Fix: translate the text to English." >&2
    echo "Only multibyte/wide-char TEST DATA may stay -- then add the file to" >&2
    echo "$ALLOW_FILE with a one-line reason." >&2
    exit 1
fi

echo "cjk: ok -- $(wc -l < "$tmp_allow" | tr -d ' ') files whitelisted, no unlisted CJK"
