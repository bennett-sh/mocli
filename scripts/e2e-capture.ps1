<#
End-to-end check of clipboard capture. Run after `cargo build`:

    pwsh -STA -File scripts/e2e-capture.ps1

It overwrites the clipboard for about 20 seconds, so do not copy or paste while
it runs. Text, images and file lists are restored afterwards; other clipboard
formats are not. It waits for 10 idle seconds before starting, uses a temporary
config and history, and needs Python to read the database. Quit any running
mocli first.
#>
param([string]$Exe = "$PSScriptRoot/../target/debug/mocli.exe")
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Idle {
    [StructLayout(LayoutKind.Sequential)] struct LASTINPUT { public uint size, time; }
    [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LASTINPUT i);
    public static long Milliseconds() {
        var i = new LASTINPUT { size = 8 }; GetLastInputInfo(ref i);
        return unchecked((uint)Environment.TickCount - i.time);
    }
}
'@
$Forms = [System.Windows.Forms.Clipboard]

if (Get-Process mocli -ErrorAction SilentlyContinue) { 'SKIPPED quit the running mocli first'; exit 2 }
if (Get-Process LogonUI -ErrorAction SilentlyContinue) { 'SKIPPED the screen is locked'; exit 2 }
$deadline = [DateTime]::Now.AddMinutes(3)
while ([Idle]::Milliseconds() -lt 10000) {
    if ([DateTime]::Now -gt $deadline) { 'SKIPPED the user never went idle; the clipboard was not touched'; exit 2 }
    Start-Sleep -Milliseconds 500
}

function Wait-Ui([int]$ms = 600) {
    $end = [DateTime]::Now.AddMilliseconds($ms)
    while ([DateTime]::Now -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 10 }
}
# Without `$flush` this process stays the clipboard owner.
function Copy-Data($data, [bool]$flush = $false) { $Forms::SetDataObject($data, $flush, 20, 50); Wait-Ui }
function New-Data([string]$text) {
    $data = [System.Windows.Forms.DataObject]::new()
    if ($text) { $data.SetText($text) }
    $data
}
function Get-Rows {
    $code = @'
import json, sqlite3, sys
db = sqlite3.connect(sys.argv[1]); db.row_factory = sqlite3.Row
rows = [dict(r) for r in db.execute("SELECT *, hex(hash) AS hex FROM entries ORDER BY id")]
for r in rows:
    r.pop("hash")
    r["formats"] = ",".join(n for n, in db.execute("SELECT name FROM formats WHERE entry_id=? ORDER BY name", (r["id"],)))
print(json.dumps(rows))
'@
    @($code | python - "$root/data/history.db" | ConvertFrom-Json)
}
function Check([string]$name, [bool]$ok, $detail = '') {
    if ($ok) { "PASS  $name" } else { $script:failed++; "FAIL  $name  $detail" }
}
function Start-Mocli {
    $process = Start-Process $Exe -PassThru -WindowStyle Hidden -RedirectStandardError "$root/stderr.txt"
    Wait-Ui 2500
    if ($process.HasExited) { throw "mocli exited: $(Get-Content "$root/stderr.txt" -Raw)" }
    $process
}

$backup = [System.Windows.Forms.DataObject]::new()
$hasBackup = $false
if ($Forms::ContainsText()) { $backup.SetText($Forms::GetText()); $hasBackup = $true }
if ($Forms::ContainsImage()) { $backup.SetImage($Forms::GetImage()); $hasBackup = $true }
if ($Forms::ContainsFileDropList()) { $backup.SetFileDropList($Forms::GetFileDropList()); $hasBackup = $true }

$root = Join-Path ([System.IO.Path]::GetTempPath()) "mocli-e2e-$PID"
New-Item -ItemType Directory -Force "$root/appdata/mocli", "$root/files" | Out-Null
$env:APPDATA = "$root/appdata"
$env:MOCLI_DATA_DIR = "$root/data"
$env:MOCLI_LOG = 'warn,mocli=debug'
$script:failed = 0
$app = $null
try {
    # The checks read the database, so it stays unencrypted.
    "[privacy]`nencryption = `"off`"" | Set-Content "$root/appdata/mocli/config.toml"
    Copy-Data (New-Data 'copied before mocli started') $true
    $app = Start-Mocli

    $data = New-Data "  plain text`r`nsecond line"
    $data.SetText('<b>plain text</b>', [System.Windows.Forms.TextDataFormat]::Html)
    Copy-Data $data
    Copy-Data (New-Data 'https://example.com/a?b=c')
    Copy-Data (New-Data 'someone@example.com')
    Copy-Data (New-Data '#ff7a1a')

    $bitmap = [System.Drawing.Bitmap]::new(300, 200)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([System.Drawing.Color]::FromArgb(255, 10, 120, 200)); $graphics.Dispose()
    foreach ($i in 1..2) {
        $data = New-Data ''; $data.SetImage($bitmap); Copy-Data $data
    }

    $small = [System.Drawing.Bitmap]::new(8, 6)
    $small.SetPixel(0, 0, [System.Drawing.Color]::FromArgb(128, 255, 0, 0))
    $png = [System.IO.MemoryStream]::new()
    $small.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
    $data = New-Data ''; $data.SetData('PNG', $false, $png); Copy-Data $data

    'one' | Set-Content "$root/files/one.txt" -NoNewline
    'second' | Set-Content "$root/files/two.txt" -NoNewline
    foreach ($names in @('one.txt'), @('one.txt', 'two.txt')) {
        $list = [System.Collections.Specialized.StringCollection]::new()
        foreach ($name in $names) { [void]$list.Add([System.IO.Path]::GetFullPath("$root/files/$name")) }
        $data = New-Data ''; $data.SetFileDropList($list); Copy-Data $data
    }

    foreach ($marker in 'ExcludeClipboardContentFromMonitorProcessing', 'Clipboard Viewer Ignore', 'CanIncludeInClipboardHistory') {
        $data = New-Data "secret $marker"
        $data.SetData($marker, $false, [System.IO.MemoryStream]::new([byte[]](0, 0, 0, 0)))
        Copy-Data $data
    }
    $data = New-Data 'history allowed'
    $data.SetData('CanIncludeInClipboardHistory', $false, [System.IO.MemoryStream]::new([byte[]](1, 0, 0, 0)))
    Copy-Data $data

    Copy-Data (New-Data "   `r`n  ")
    Copy-Data (New-Data 'https://example.com/a?b=c')
    Copy-Data (New-Data 'flushed copy') $true
    Wait-Ui 500

    $rows = Get-Rows
    $summary = $rows | ForEach-Object { "$($_.kind): $($_.title) x$($_.copy_count)" }
    function Row($title) { $rows | Where-Object title -CEQ $title | Select-Object -First 1 }

    Check 'earlier clipboard content is not captured' (-not (Row 'copied before mocli started'))
    $text = Row 'plain text'
    Check 'text is stored unchanged with HTML' ($text.kind -eq 'text' -and $text.text -ceq "  plain text`r`nsecond line" -and $text.formats -eq 'html') ($text | ConvertTo-Json -Compress)
    Check 'source app is this PowerShell' ($text.source_exe -like '*\pwsh.exe' -and $text.source_name) "$($text.source_exe) / $($text.source_name)"
    "INFO  source: $($text.source_name) | title: $($text.source_title)"
    Check 'link kind' ((Row 'https://example.com/a?b=c').kind -eq 'link')
    Check 'email kind' ((Row 'someone@example.com').kind -eq 'email')
    Check 'color kind' ((Row '#ff7a1a').kind -eq 'color')
    Check 'recopy bumps the entry' ((Row 'https://example.com/a?b=c').copy_count -eq 2)
    Check 'single copies count once' ((Row 'someone@example.com').copy_count -eq 1 -and $text.copy_count -eq 1)

    $image = Row 'Image (300×200)'
    Check 'bitmap is stored once with two copies' ($image.kind -eq 'image' -and $image.width -eq 300 -and $image.copy_count -eq 2) ($summary -join ' | ')
    if ($image) {
        $blob = "$root/data/blobs/$($image.hex.ToLower())"
        $stored = [System.Drawing.Bitmap]::new("$blob.png")
        $pixel = $stored.GetPixel(5, 5); $stored.Dispose()
        Check 'stored image has the right pixels' ($pixel.A -eq 255 -and $pixel.R -eq 10 -and $pixel.G -eq 120 -and $pixel.B -eq 200) "$pixel"
        $thumb = [System.Drawing.Bitmap]::new("$blob.thumb.png")
        Check 'thumbnail is scaled' ($thumb.Width -eq 128 -and $thumb.Height -eq 85) "$($thumb.Width)x$($thumb.Height)"
        $thumb.Dispose()
    }
    $alpha = Row 'Image (8×6)'
    if ($alpha) {
        $stored = [System.Drawing.Bitmap]::new("$root/data/blobs/$($alpha.hex.ToLower()).png")
        $a = $stored.GetPixel(0, 0).A; $stored.Dispose()
    }
    Check 'PNG keeps transparency' ($alpha -and $a -eq 128) "alpha=$a"

    $one = Row 'one.txt'
    Check 'single file' ($one.kind -eq 'files' -and $one.size_bytes -eq 3 -and $one.text -like '*one.txt') ($one | ConvertTo-Json -Compress)
    $two = Row '2 files'
    Check 'several files' ($two.kind -eq 'files' -and $two.size_bytes -eq 9 -and @($two.text -split "`n").Count -eq 2) ($two | ConvertTo-Json -Compress)

    Check 'private content is never stored' (-not ($rows | Where-Object title -Like 'secret*')) ($summary -join ' | ')
    Check 'explicitly allowed content is stored' ([bool](Row 'history allowed'))
    Check 'flushed copy is captured' ([bool](Row 'flushed copy'))
    Check 'nothing unexpected was stored' ($rows.Count -eq 10) "$($rows.Count): $($summary -join ' | ')"

    Stop-Process -Id $app.Id; $app.WaitForExit()
    "[privacy]`nencryption = `"off`"`nignore_apps = [`"PWSH.exe`"]" | Set-Content "$root/appdata/mocli/config.toml"
    $app = Start-Mocli
    Copy-Data (New-Data 'from an ignored app')
    $after = Get-Rows
    Check 'ignored apps are not captured' ($after.Count -eq $rows.Count) "$($after.Count)"
    Check 'history survives a restart' ([bool]($after | Where-Object title -CEQ 'plain text'))
}
catch { $script:failed++; "ERROR $_" }
finally {
    if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id; $app.WaitForExit() }
    if ($hasBackup) { $Forms::SetDataObject($backup, $true, 20, 50) } else { $Forms::Clear() }
}
"---- $script:failed failed"
'---- warnings and errors in log'
Get-ChildItem "$root/data/logs" -ErrorAction SilentlyContinue | Get-Content | Select-String ' WARN | ERROR '
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
exit [int]($script:failed -gt 0)
