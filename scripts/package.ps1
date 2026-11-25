<#
Builds the release and packages it into dist/: a portable zip, an installer and
their checksums.

    pwsh scripts/package.ps1                 # everything
    pwsh scripts/package.ps1 -SkipInstaller  # without Inno Setup

The installer needs Inno Setup 6: winget install JRSoftware.InnoSetup
#>
param(
    [switch]$SkipBuild,
    [switch]$SkipInstaller
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try {
    $package = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages |
        Where-Object name -EQ 'mocli'
    $version = $package.version
    if (-not $SkipBuild) { cargo build --release --locked }
    $exe = Join-Path $repo 'target/release/mocli.exe'
    if (-not (Test-Path $exe)) { throw "missing $exe" }

    $dist = Join-Path $repo 'dist'
    if (Test-Path $dist) { Remove-Item -Recurse -Force $dist }
    $stage = Join-Path $dist 'portable'
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item $exe $stage
    # Its presence keeps settings and history next to the program.
    Set-Content (Join-Path $stage 'portable') ''
    @"
mocli $version (portable)

Run mocli.exe and press Win+V. Settings and history are kept in the "data"
folder next to it. Delete the "portable" file to use your user profile instead.

https://github.com/bennett-sh/mocli
"@ | Set-Content (Join-Path $stage 'README.txt')
    $zip = Join-Path $dist "mocli-$version-x64-portable.zip"
    Compress-Archive -Path "$stage/*" -DestinationPath $zip
    Remove-Item -Recurse -Force $stage

    if (-not $SkipInstaller) {
        $compiler = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
        if (-not $compiler) {
            $compiler = @(
                "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
                "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
                "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
            ) | Where-Object { Test-Path $_ } | Select-Object -First 1
        }
        if (-not $compiler) {
            throw 'Inno Setup 6 was not found. Install it, or pass -SkipInstaller.'
        }
        & $compiler /Qp "/DAppVersion=$version" "/DSourceExe=$exe" "$repo/installer/mocli.iss"
    }

    Get-ChildItem $dist -File | ForEach-Object {
        "$((Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower())  $($_.Name)"
    } | Set-Content (Join-Path $dist 'SHA256SUMS.txt')
    Get-ChildItem $dist -File | ForEach-Object { "{0,12:N0}  {1}" -f $_.Length, $_.Name }
}
finally { Pop-Location }
