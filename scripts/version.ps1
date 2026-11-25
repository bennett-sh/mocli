# Prints the release version from [package] in Cargo.toml.
$ErrorActionPreference = 'Stop'
$manifest = Join-Path (Split-Path $PSScriptRoot -Parent) 'Cargo.toml'
$text = Get-Content $manifest -Raw
if ($text -match '(?ms)^\[package\]\s*$.*?^version\s*=\s*"(?<v>\d+\.\d+\.\d+[^"]*)"') {
    $Matches['v']
} else {
    throw "No version found in [package] of $manifest"
}
