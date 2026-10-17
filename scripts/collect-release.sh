#!/usr/bin/env bash
# Put the built programs into a folder in the release layout:
#
#   Passport Scanner(.exe)   the desktop program
#   tools/, debug-tools/     the others, as scripts/release-layout.txt places them
#   LICENSE, licenses/
#
#   scripts/collect-release.sh dist
#
# Fails on a program the build made that the layout does not place, so a new
# one cannot go missing from releases unnoticed.
set -euo pipefail

dist=${1:?usage: collect-release.sh <folder>}
root=$(cd "$(dirname "$0")/.." && pwd)
bin="$root/target/release"
exe=""
[ -f "$bin/passport-scanner-desktop.exe" ] && exe=".exe"

mkdir -p "$dist"
cp "$bin/passport-scanner-desktop$exe" "$dist/Passport Scanner$exe"
placed=" passport-scanner-desktop "
while read -r folder program; do
    program=${program%$'\r'}
    case "$folder" in '' | '#'*) continue ;; esac
    placed+="$program "
    # The browser client's native build, for one: it runs in a browser.
    [ "$folder" = "-" ] && continue
    mkdir -p "$dist/$folder"
    cp "$bin/$program$exe" "$dist/$folder/"
done < "$root/scripts/release-layout.txt"

for file in "$bin"/passport-scanner-*; do
    [ -f "$file" ] || continue
    case "$file" in *.d | *.pdb) continue ;; esac
    name=$(basename "$file" "$exe")
    case "$placed" in
        *" $name "*) ;;
        *)
            echo "error: $name is built but scripts/release-layout.txt does not place it" >&2
            exit 1
            ;;
    esac
done

cp "$root/LICENSE" "$dist/"
mkdir -p "$dist/licenses"
cp -r "$root/licenses/." "$dist/licenses/"
