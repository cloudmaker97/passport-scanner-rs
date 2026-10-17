#!/usr/bin/env bash
# Build the user guide PDF from docs/user/, with the full licence appended.
#
#   scripts/build-user-guide.sh [output.pdf]
#
# Needs pandoc, xelatex (texlive-xetex, texlive-latex-extra) and DejaVu fonts.
# VERSION, when set, is printed on the title page.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/dist/Passport-Scanner-User-Guide.pdf}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The license must travel with the program; the guide carries it verbatim.
{
    printf '# MIT License {.unnumbered}\n\n```\n'
    cat "$root/LICENSE"
    printf '```\n'
} > "$tmp/99-license.md"

mkdir -p "$(dirname "$out")"
pandoc "$root/docs/user/metadata.yaml" "$root"/docs/user/[0-9]*.md "$tmp/99-license.md" \
    --pdf-engine=xelatex \
    --resource-path="$root/docs/user" \
    ${VERSION:+--metadata=date:"Version $VERSION"} \
    -o "$out"
echo "$out"
