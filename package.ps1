# Build the programs and put them in dist\ ready to run.
#
#   .\package.ps1
#
# The service serves the browser client from www\ beside itself: the shared
# Iced interface compiled to WebAssembly. That needs, once:
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock>
#
# wasm-opt (from binaryen) is used when installed and shrinks the module.

$ErrorActionPreference = "Stop"
$root = $PSScriptRoot
$dist = Join-Path $root "dist"
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoExecutable = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe" }
if (-not (Test-Path -LiteralPath $cargoExecutable)) { throw "Rust cargo was not found. Install a Rust toolchain first." }

Write-Host "building..." -ForegroundColor Cyan
Push-Location $root
try {
    # Cargo writes its progress to stderr, and with ErrorActionPreference set
    # to Stop that alone aborts the script even on a build that succeeded. The
    # exit code is the thing to believe.
    $ErrorActionPreference = "Continue"
    & $cargoExecutable build --release -p passport-scanner-app -p passport-scanner-desktop
    $built = $LASTEXITCODE
    if ($built -ne 0) { throw "the build failed" }
    & $cargoExecutable build --release --target wasm32-unknown-unknown -p passport-scanner-webui
    $built = $LASTEXITCODE
    $ErrorActionPreference = "Stop"
    if ($built -ne 0) { throw "the web client build failed (rustup target add wasm32-unknown-unknown)" }
} finally {
    Pop-Location
}

# A running service holds its own executable open, so say so plainly rather
# than failing on an access error the reader has to decode.
$running = Get-Process -Name "passport-scanner-*", "Passport Scanner" -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -and $_.Path.StartsWith($dist) }
if ($running) {
    $names = ($running | ForEach-Object { $_.Name }) -join ", "
    throw "$names is running from dist\ and holds the file open. Close it, then run this again."
}

# wasm-bindgen's generated glue must come from the exact library version the
# module was linked against; check before producing a client that cannot load.
$lock = Get-Content (Join-Path $root "Cargo.lock") -Raw
$expected = [regex]::Match($lock, '(?m)^name = "wasm-bindgen"\r?\nversion = "([^"]+)"').Groups[1].Value
$bindgen = Get-Command wasm-bindgen -ErrorAction SilentlyContinue
if (-not $bindgen) { throw "wasm-bindgen was not found. Run: cargo install wasm-bindgen-cli --version $expected" }
$installed = ((& $bindgen.Source --version) -split " ")[1]
if ($installed -ne $expected) {
    throw "wasm-bindgen $installed does not match $expected from Cargo.lock. Run: cargo install wasm-bindgen-cli --version $expected"
}

# Replace only packaged files; keep any user files placed in dist.
$www = Join-Path $dist "www"
New-Item -ItemType Directory -Force -Path $www | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $dist "licenses") | Out-Null
# The former HTML interface is superseded by www\.
Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $dist "ui")
# Programs the former flat layout kept beside the desktop program.
Get-ChildItem $dist -Filter "passport-scanner-*.exe" -File -ErrorAction SilentlyContinue | Remove-Item -Force

$module = Join-Path $root "target\wasm32-unknown-unknown\release\passport-scanner-webui.wasm"
& $bindgen.Source --target web --no-typescript --out-dir $www $module
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }
$optimizer = Get-Command wasm-opt -ErrorAction SilentlyContinue
if ($optimizer) {
    $output = Join-Path $www "passport-scanner-webui_bg.wasm"
    & $optimizer.Source -Oz --enable-bulk-memory --enable-nontrapping-float-to-int -o $output $output
    if ($LASTEXITCODE -ne 0) { throw "wasm-opt failed" }
}
Copy-Item (Join-Path $root "crates\passport-scanner-webui\index.html") $www

# "Passport Scanner" at the top, the other programs where
# scripts\release-layout.txt puts them - the same layout as a release.
$release = Join-Path $root "target\release"
Copy-Item (Join-Path $release "passport-scanner-desktop.exe") (Join-Path $dist "Passport Scanner.exe")
foreach ($line in Get-Content (Join-Path $root "scripts\release-layout.txt")) {
    $fields = $line.Trim() -split "\s+"
    if ($fields.Count -lt 2 -or $fields[0].StartsWith("#") -or $fields[0] -eq "-") { continue }
    $folder = Join-Path $dist $fields[0]
    New-Item -ItemType Directory -Force -Path $folder | Out-Null
    Copy-Item (Join-Path $release "$($fields[1]).exe") $folder
}
Copy-Item (Join-Path $root "LICENSE.txt") $dist
Copy-Item (Join-Path $root "licenses\*") (Join-Path $dist "licenses")

Write-Host "`ndist\" -ForegroundColor Green
Get-ChildItem $dist -Recurse -File | ForEach-Object {
    "  {0,-44} {1,12:N0} bytes" -f $_.FullName.Replace("$dist\", ""), $_.Length
}
Write-Host "`nstart the desktop with:  & '.\dist\Passport Scanner.exe'" -ForegroundColor Cyan
