#!/usr/bin/env bash
# Build the user guide PDF from docs/user/, with the full licence appended.
#
#   scripts/build-user-guide.sh [output.pdf]
#
# Needs pandoc (3.x) and typst. The fonts come from docs/user/fonts.
# VERSION, when set, is printed on the cover and in the footer.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/dist/Passport-Scanner-User-Guide.pdf}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The license must travel with the program; the guide carries it verbatim.
{
    printf '# MIT License {.unnumbered}\n\n```license\n'
    cat "$root/LICENSE.txt"
    printf '```\n'
} > "$tmp/99-license.md"

cp "$root/docs/user/handbook.typ" "$root/icon.svg" "$tmp/"
# --columns=1 gives every table the full text width, split by its dashes.
pandoc "$root/docs/user/metadata.yaml" "$root"/docs/user/[0-9]*.md "$tmp/99-license.md" \
    --to=typst --standalone --columns=1 --wrap=none \
    --template="$root/docs/user/template.typ" \
    ${VERSION:+--variable=version:"Version $VERSION"} \
    -o "$tmp/guide.typ"

mkdir -p "$(dirname "$out")"
typst compile --ignore-system-fonts --font-path "$root/docs/user/fonts" \
    "$tmp/guide.typ" "$out"
echo "$out"
