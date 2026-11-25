# Generates a markdown changelog from conventional commits ("fix(popup): ...", "feat: ...").
#
#   ./scripts/changelog.ps1 -From v0.1.0            # commits v0.1.0..HEAD
#   ./scripts/changelog.ps1                         # every commit up to HEAD
#
# "chore: bump version" commits and merge commits are left out. Commits that do not follow
# the convention are listed under "Other changes".
param(
    [string]$From = '',
    [string]$To = 'HEAD'
)
$ErrorActionPreference = 'Stop'

$range = if ($From) { "$From..$To" } else { $To }
$log = git log $range --no-merges --pretty=format:'%h%x09%s'
if ($LASTEXITCODE -ne 0) { throw "git log failed for range $range" }

# type -> section title, in display order
$sections = [ordered]@{
    breaking = 'Breaking changes'
    feat     = 'Features'
    fix      = 'Fixes'
    perf     = 'Performance'
    refactor = 'Refactoring'
    docs     = 'Documentation'
    test     = 'Tests'
    build    = 'Build'
    ci       = 'CI'
    chore    = 'Chores'
    style    = 'Style'
    other    = 'Other changes'
}
$entries = @{}
foreach ($key in $sections.Keys) { $entries[$key] = [System.Collections.Generic.List[string]]::new() }

foreach ($line in @($log)) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $hash, $subject = $line -split "`t", 2
    if ($subject -match '^chore(\([^)]*\))?!?:\s*bump version') { continue }

    if ($subject -match '^(?<type>[a-z]+)(\((?<scope>[^)]+)\))?(?<bang>!)?:\s*(?<desc>.+)$') {
        $type = $Matches['type']
        $scope = $Matches['scope']
        $desc = $Matches['desc'].Trim()
        $breaking = [bool]$Matches['bang']
    } else {
        $type = 'other'; $scope = ''; $desc = $subject.Trim(); $breaking = $false
    }
    if (-not $sections.Contains($type)) { $type = 'other' }

    $prefix = if ($scope) { "**${scope}:** " } else { '' }
    $item = "- $prefix$desc (``$hash``)"
    if ($breaking) { $entries['breaking'].Add($item) } else { $entries[$type].Add($item) }
}

$out = [System.Collections.Generic.List[string]]::new()
foreach ($key in $sections.Keys) {
    if ($entries[$key].Count -eq 0) { continue }
    $out.Add("### $($sections[$key])")
    $out.Add('')
    $out.AddRange($entries[$key])
    $out.Add('')
}
if ($out.Count -eq 0) { $out.Add('No notable changes.') }
$out -join "`n"
