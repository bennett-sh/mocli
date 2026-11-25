<#
End-to-end check of the popup: search, keyboard and mouse use, and pasting.
Run after `cargo build`:

    pwsh -STA -File scripts/e2e-popup.ps1

It types into its own test window and overwrites the clipboard, so leave the
keyboard and mouse alone while it runs (about a minute). It waits for 15 idle
seconds before starting, and uses a temporary config and history. Text, images
and file lists on the clipboard are restored; other formats are not. Needs
Python to read the database. Quit any running mocli first.
#>
param(
    [string]$Exe = "$PSScriptRoot/../target/debug/mocli.exe",
    [string]$Shots = "$env:TEMP/mocli-shots"
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class N {
    [StructLayout(LayoutKind.Sequential)] struct LASTINPUT { public uint size, time; }
    [DllImport("user32.dll")] static extern bool GetLastInputInfo(ref LASTINPUT i);
    public static long IdleMs() {
        var i = new LASTINPUT { size = 8 }; GetLastInputInfo(ref i);
        return unchecked((uint)Environment.TickCount - i.time);
    }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct KI { public ushort vk, scan; public uint flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct MI { public int x, y; public uint data, flags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct U { [FieldOffset(0)] public KI k; [FieldOffset(0)] public MI m; }
    [StructLayout(LayoutKind.Sequential)] struct INPUT { public uint type; public U u; }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] i, int size);
    [DllImport("user32.dll")] static extern uint MapVirtualKey(uint code, uint type);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern short VkKeyScan(char c);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public static IntPtr VisibleWindow(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); RECT r; GetWindowRect(h, out r);
            if (p == pid && IsWindowVisible(h) && r.R - r.L > 300) { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    }
    public static void Key(ushort vk, bool up) {
        var i = new INPUT { type = 1 };
        i.u.k = new KI { vk = vk, scan = (ushort)MapVirtualKey(vk, 0), flags = (up ? 2u : 0u) | (vk == 0xA5 || (vk >= 0x21 && vk <= 0x28) ? 1u : 0u) };
        SendInput(1, new[] { i }, Marshal.SizeOf(typeof(INPUT)));
    }
    public static void Click() {
        var d = new INPUT { type = 0 }; d.u.m = new MI { flags = 2 };
        var u = new INPUT { type = 0 }; u.u.m = new MI { flags = 4 };
        SendInput(2, new[] { d, u }, Marshal.SizeOf(typeof(INPUT)));
    }
}
'@
[void][N]::SetProcessDpiAwarenessContext([IntPtr](-4))
$Forms = [System.Windows.Forms.Clipboard]

function Wait-Ui([int]$ms = 120) {
    $end = [DateTime]::Now.AddMilliseconds($ms)
    while ([DateTime]::Now -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 5 }
}
function Assert-Safe {
    if ([N]::GetForegroundWindow() -ne $form.Handle) { throw "ABORT: test window lost the foreground" }
}
# Send-Chord 0xA4,0x56 = hold Alt, press V, release in reverse order.
function Send-Chord([int[]]$vks, [int]$wait = 120) {
    Assert-Safe
    foreach ($vk in $vks) { [N]::Key($vk, $false); Wait-Ui 15 }
    [array]::Reverse($vks)
    foreach ($vk in $vks) { [N]::Key($vk, $true); Wait-Ui 15 }
    Wait-Ui $wait
}
function Send-Text([string]$text) {
    foreach ($c in $text.ToCharArray()) {
        $s = [N]::VkKeyScan($c); $vk = $s -band 0xFF
        if (($s -shr 8) -band 1) { Send-Chord @(0xA0, $vk) 30 } else { Send-Chord @($vk) 30 }
    }
    Wait-Ui 150
}
function Click-At([int]$x, [int]$y) { [void][N]::SetCursorPos($x, $y); Wait-Ui 50; [N]::Click(); Wait-Ui 300 }
function Get-Log { Get-Content "$env:MOCLI_DATA_DIR\logs\mocli.*.log" -ErrorAction SilentlyContinue }
function Check([string]$name, [bool]$ok, $detail = '') {
    if ($ok) { "PASS  $name" } else { $script:failed++; "FAIL  $name  $detail" }
}
function Popup { [N]::VisibleWindow($app.Id) }
function Closed { (Popup) -eq [IntPtr]::Zero }
function Open-Popup { Send-Chord @(0xA4, 0x56) 600 }
function Bounds { $r = New-Object N+RECT; [void][N]::GetWindowRect((Popup), [ref]$r); $r }
function Save-Shot([string]$name) {
    Wait-Ui 400
    $r = Bounds
    $bmp = New-Object System.Drawing.Bitmap (($r.R - $r.L), ($r.B - $r.T))
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size); $bmp.Save("$Shots/$name.png"); $g.Dispose(); $bmp.Dispose()
}
# Clicks a point given in the popup's logical pixels (760 wide).
function Click-Popup([double]$x, [double]$y, [int]$wait = 300) {
    $r = Bounds; $scale = ($r.R - $r.L) / 760.0
    [void][N]::SetCursorPos([int]($r.L + $x * $scale), [int]($r.T + $y * $scale)); Wait-Ui 50; [N]::Click(); Wait-Ui $wait
}
# This process stays the clipboard owner, so it is recorded as the source.
function Copy-Data($data) { $Forms::SetDataObject($data, $false, 20, 50); Wait-Ui 500 }
function New-Data([string]$text) {
    $data = [System.Windows.Forms.DataObject]::new()
    if ($text) { $data.SetText($text) }
    $data
}
function Get-Rows {
    $code = @'
import json, sqlite3, sys
db = sqlite3.connect(sys.argv[1]); db.row_factory = sqlite3.Row
print(json.dumps([dict(r) for r in db.execute(
    "SELECT title, kind, copy_count, last_pasted FROM entries ORDER BY last_copied DESC, id DESC")]))
'@
    @($code | python - "$root/data/history.db" | ConvertFrom-Json)
}
# Empties the box, runs the keys with the popup open, and returns what arrived.
function Use-Popup([scriptblock]$keys) {
    $box.Clear(); Wait-Ui 100
    Open-Popup
    . $keys | Out-Null
    Wait-Ui 600
    $box.Text
}

if (Get-Process mocli -ErrorAction SilentlyContinue) { 'SKIPPED quit the running mocli first'; exit 2 }
$deadline = [DateTime]::Now.AddMinutes(4)
while ([N]::IdleMs() -lt 15000) {
    if ([DateTime]::Now -gt $deadline) { 'SKIPPED the user never went idle; nothing was touched'; exit 2 }
    Start-Sleep -Milliseconds 500
}

$backup = [System.Windows.Forms.DataObject]::new()
$hasBackup = $false
if ($Forms::ContainsText()) { $backup.SetText($Forms::GetText()); $hasBackup = $true }
if ($Forms::ContainsImage()) { $backup.SetImage($Forms::GetImage()); $hasBackup = $true }
if ($Forms::ContainsFileDropList()) { $backup.SetFileDropList($Forms::GetFileDropList()); $hasBackup = $true }

$root = Join-Path ([System.IO.Path]::GetTempPath()) "mocli-popup-$PID"
New-Item -ItemType Directory -Force "$root/appdata", "$root/files", $Shots | Out-Null
$env:APPDATA = "$root/appdata"
$env:MOCLI_DATA_DIR = "$root/data"
$env:MOCLI_LOG = 'warn,mocli=debug'
$script:failed = 0
$app = $null; $form = $null; $other = $null; $stripes = $null
try {
    $app = Start-Process $Exe -PassThru -WindowStyle Hidden -RedirectStandardError "$root/stderr.txt"
    Wait-Ui 2500
    Check 'app is running and ready' ((-not $app.HasExited) -and (Get-Log | Select-String 'ready'))
    $second = Start-Process $Exe -PassThru -WindowStyle Hidden -RedirectStandardError "$root/stderr2.txt"
    Check 'second instance exits' ($second.WaitForExit(5000))

    # History, oldest first.
    Copy-Data (New-Data 'alpha apples')
    $data = New-Data 'bravo bananas'
    $data.SetText('<b>bravo bananas</b>', [System.Windows.Forms.TextDataFormat]::Html)
    Copy-Data $data
    Copy-Data (New-Data '#ff7a1a')
    Copy-Data (New-Data 'https://example.com/cherry')
    $bitmap = [System.Drawing.Bitmap]::new(480, 300)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.Clear([System.Drawing.Color]::FromArgb(255, 10, 120, 200))
    $graphics.FillEllipse([System.Drawing.Brushes]::Gold, 140, 50, 200, 200); $graphics.Dispose()
    $data = New-Data ''; $data.SetImage($bitmap); Copy-Data $data
    'file content' | Set-Content "$root/files/report.txt" -NoNewline
    $file = [System.IO.Path]::GetFullPath("$root/files/report.txt")
    $list = [System.Collections.Specialized.StringCollection]::new(); [void]$list.Add($file)
    $data = New-Data ''; $data.SetFileDropList($list); Copy-Data $data
    Copy-Data (New-Data 'delta dates')
    Check 'seven entries are recorded' (@(Get-Rows).Count -eq 7) "$(@(Get-Rows).Count)"

    # Colored stripes behind the popup show whether the backdrop blurs them.
    $stripes = New-Object System.Windows.Forms.Form
    $stripes.FormBorderStyle = 'None'; $stripes.StartPosition = 'Manual'
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $stripes.Bounds = $screen
    $colors = 'Red','White','Lime','Blue','Yellow','Magenta'
    for ($i = 0; $i -lt 24; $i++) {
        $panel = New-Object System.Windows.Forms.Panel
        $panel.BackColor = [System.Drawing.Color]::FromName($colors[$i % 6])
        $panel.SetBounds([int]($i * $screen.Width / 24), 0, [int]($screen.Width / 24) + 1, $screen.Height)
        $stripes.Controls.Add($panel)
    }
    $stripes.Show(); Wait-Ui 300

    $form = New-Object System.Windows.Forms.Form
    $form.Text = 'mocli e2e target'; $form.StartPosition = 'Manual'; $form.TopMost = $true
    $form.Location = '20,20'; $form.Size = '420,140'
    $box = New-Object System.Windows.Forms.TextBox; $box.Dock = 'Top'; $form.Controls.Add($box)
    $form.Show(); Wait-Ui 300
    $p = $box.PointToScreen([System.Drawing.Point]::new(100, 8)); Click-At $p.X $p.Y
    Assert-Safe
    Send-Text 'abc'
    Check 'baseline typing reaches the text box' ($box.Text -ceq 'abc') $box.Text

    Open-Popup
    Check 'Alt+V shows the popup' (-not (Closed))
    Check 'target keeps foreground and focus' (([N]::GetForegroundWindow() -eq $form.Handle) -and $box.Focused)
    $ex = [N]::GetWindowLong((Popup), -20)
    Check 'popup has NOACTIVATE and TOPMOST' ((($ex -band 0x08000000) -ne 0) -and (($ex -band 8) -ne 0))
    $wr = Bounds; $cr = New-Object N+RECT; [void][N]::GetClientRect((Popup), [ref]$cr)
    Check 'popup has no invisible frame' ((($wr.R - $wr.L) -eq $cr.R) -and (($wr.B - $wr.T) -eq $cr.B))
    Save-Shot 'list'
    Send-Chord @(0x28); Save-Shot 'file'
    Send-Chord @(0x28); Save-Shot 'image'
    Send-Text 'banana'; Save-Shot 'search'
    Check 'typed keys do not reach the text box' ($box.Text -ceq 'abc') $box.Text
    Send-Chord @(0x1B); Send-Chord @(0x1B) 400
    Check 'Escape clears the search, then closes' (Closed)

    $got = Use-Popup { Send-Chord @(0x0D) }
    Check 'Enter pastes the newest entry' ($got -ceq 'delta dates') $got
    Check 'popup closes after pasting' (Closed)

    $got = Use-Popup { Send-Text 'banana'; Send-Chord @(0x0D) }
    Check 'search then Enter pastes the match' ($got -ceq 'bravo bananas') $got
    Check 'paste restores HTML' ($Forms::ContainsText([System.Windows.Forms.TextDataFormat]::Html))
    $rows = Get-Rows
    $bravo = $rows | Where-Object title -CEQ 'bravo bananas'
    Check 'pasted entry moves to the top' ($rows[0].title -ceq 'bravo bananas' -and $bravo.last_pasted) $rows[0].title
    Check 'our own paste is not captured again' ($bravo.copy_count -eq 1 -and $rows.Count -eq 7) "$($bravo.copy_count), $($rows.Count)"

    $got = Use-Popup { Send-Chord @(0x28); Send-Chord @(0x0D) }
    Check 'arrow down then Enter pastes the second entry' ($got -ceq 'delta dates') $got

    $got = Use-Popup { Send-Text 'banana'; Send-Chord @(0xA0, 0x0D) }
    Check 'Shift+Enter pastes' ($got -ceq 'bravo bananas') $got
    Check 'Shift+Enter drops formatting' (-not $Forms::ContainsText([System.Windows.Forms.TextDataFormat]::Html))
    Send-Text 'x'
    Check 'no modifier is left stuck' ($box.Text -ceq 'bravo bananasx') $box.Text

    $got = Use-Popup { Send-Text 'apple'; Send-Chord @(0xA2, 0x0D) }
    Check 'Ctrl+Enter copies without pasting' ($got -eq '' -and $Forms::GetText() -ceq 'alpha apples' -and (Closed)) "box=$got clipboard=$($Forms::GetText())"

    $got = Use-Popup { Send-Text 'alphapples'; Send-Chord @(0x0D) }
    Check 'a loose match can be pasted' ($got -ceq 'alpha apples') $got

    $got = Use-Popup { Send-Chord @(0x09); Send-Chord @(0x09); Save-Shot 'images'; Send-Chord @(0xA2, 0x0D) }
    $image = $Forms::GetImage()
    $pixel = if ($image) { $image.GetPixel(5, 5) }
    Check 'Tab filters to images; copy restores the picture' ($image.Width -eq 480 -and $pixel.R -eq 10 -and $pixel.B -eq 200) "$($image.Width) $pixel"
    Check 'image copy also offers PNG' ($Forms::ContainsData('PNG'))

    $got = Use-Popup { Send-Chord @(0x09); Send-Chord @(0x09); Send-Chord @(0x09); Send-Chord @(0xA2, 0x0D) }
    $dropped = $Forms::GetFileDropList()
    Check 'copying a file entry restores the file list' ($dropped.Count -eq 1 -and $dropped[0] -ceq $file) "$dropped"

    # The dropdown takes focus; afterwards typing must search again.
    $got = Use-Popup {
        Click-Popup 660 30 500; Save-Shot 'dropdown'
        Send-Text 'link'; Send-Chord @(0x0D) 500; Save-Shot 'links'
        Send-Text 'cher'; Send-Chord @(0x0D)
    }
    Check 'dropdown selects a type and search keeps working' ($got -ceq 'https://example.com/cherry') $got
    $got = Use-Popup { Click-Popup 660 30 500; Send-Chord @(0x1B) 500; Send-Text 'delta'; Send-Chord @(0x0D) }
    Check 'closing the dropdown returns to the search' ($got -ceq 'delta dates') $got

    # The second row: below the header, the section title and one 40px row.
    $box.Clear(); Open-Popup
    Click-Popup 150 150 700
    Save-Shot 'clicked'
    Check 'click selects without pasting or stealing focus' ($box.Text -eq '' -and -not (Closed) -and $box.Focused) $box.Text
    Click-Popup 150 150 120; Click-Popup 150 150 600
    Check 'double click pastes and closes' ((Closed) -and $box.Text -ne '') $box.Text

    $got = Use-Popup { Send-Text 'zzzz'; Send-Chord @(0x0D); Save-Shot 'empty' }
    Check 'Enter with no results does nothing' ($got -eq '' -and -not (Closed)) $got
    Copy-Data (New-Data 'zzzz arrived while open')
    Wait-Ui 400; Send-Chord @(0x0D) 600
    Check 'new copies appear while open' ($box.Text -ceq 'zzzz arrived while open') $box.Text

    $box.Clear(); Open-Popup; Open-Popup
    Check 'hotkey toggles closed' (Closed)
    Send-Text 'e'
    Check 'no stray menu focus after Alt+V' ($box.Text -ceq 'e') $box.Text

    # Type immediately, before the popup has had time to appear.
    $box.Clear(); Wait-Ui 100
    Send-Chord @(0xA4, 0x56) 0; Send-Text 'delta'; Send-Chord @(0x0D) 600
    Check 'type-ahead is not lost' ($box.Text -ceq 'delta dates') $box.Text

    $box.Clear(); Open-Popup
    Click-At $p.X $p.Y
    Check 'click outside closes' (Closed)
    Send-Text 'f'
    Check 'keyboard released after outside click' ($box.Text -ceq 'f') $box.Text

    Open-Popup
    $other = New-Object System.Windows.Forms.Form; $other.TopMost = $true; $other.StartPosition = 'Manual'; $other.Location = '20,200'
    $other.Show(); $other.Activate(); Wait-Ui 600
    Check 'foreground change closes' (Closed)
    $problems = @(Get-Log | Select-String ' WARN | ERROR ')
    Check 'nothing went wrong in the log' ($problems.Count -eq 0) "$($problems.Count) lines"
}
catch { $script:failed++; "ERROR $_ $($_.ScriptStackTrace)" }
finally {
    foreach ($vk in 0xA0, 0xA2, 0xA4, 0xA5) { [N]::Key($vk, $true) }
    foreach ($window in $other, $form, $stripes) { if ($window) { $window.Dispose() } }
    if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id; $app.WaitForExit() }
    if ($hasBackup) { $Forms::SetDataObject($backup, $true, 20, 50) } else { $Forms::Clear() }
}
"---- $script:failed failed"
'---- warnings and errors in log'
Get-Log | Select-String ' WARN | ERROR '
Get-Content "$root/stderr.txt" -ErrorAction SilentlyContinue | Select-String 'panic'
Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
"---- screenshots in $Shots"
exit [int]($script:failed -gt 0)
