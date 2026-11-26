# Shared helpers for the end-to-end scripts. Dot-source it, then call
# Start-Test before touching anything. Scripts set $script:target to the window
# that real input is meant for.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Text;
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
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern short VkKeyScan(char c);
    delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    public static string Title(IntPtr h) { var s = new StringBuilder(512); GetWindowText(h, s, 512); return s.ToString(); }
    public static IntPtr VisibleWindow(uint pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); RECT r; GetWindowRect(h, out r);
            if (p == pid && IsWindowVisible(h) && r.R - r.L > 300) { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    }
    public static IntPtr WindowTitled(string prefix) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => { if (IsWindowVisible(h) && Title(h).StartsWith(prefix)) { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    }
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int x, y; }
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(POINT p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    // The top-level window a click at this point would reach.
    public static IntPtr RootAt(int x, int y) { return GetAncestor(WindowFromPoint(new POINT { x = x, y = y }), 2); }
    public static void Key(ushort vk, bool up) {
        var i = new INPUT { type = 1 };
        // Arrows and friends, right Alt and the Win key are "extended".
        bool extended = vk == 0xA5 || vk == 0x5B || (vk >= 0x21 && vk <= 0x28);
        i.u.k = new KI { vk = vk, scan = (ushort)MapVirtualKey(vk, 0), flags = (up ? 2u : 0u) | (extended ? 1u : 0u) };
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
$script:failed = 0
$script:app = $null
$script:target = [IntPtr]::Zero

function Save-Clipboard {
    $script:backup = [System.Windows.Forms.DataObject]::new()
    $script:hasBackup = $false
    if ($Forms::ContainsText()) { $script:backup.SetText($Forms::GetText()); $script:hasBackup = $true }
    if ($Forms::ContainsImage()) { $script:backup.SetImage($Forms::GetImage()); $script:hasBackup = $true }
    if ($Forms::ContainsFileDropList()) { $script:backup.SetFileDropList($Forms::GetFileDropList()); $script:hasBackup = $true }
}

# Exits unless the machine has been idle, then redirects mocli to a temporary
# config and history and remembers the clipboard.
function Start-Test {
    if (Get-Process mocli -ErrorAction SilentlyContinue) { Write-Host 'SKIPPED quit the running mocli first'; exit 2 }
    $deadline = [DateTime]::Now.AddMinutes(4)
    $told = $false
    while ([N]::IdleMs() -lt 15000) {
        if (-not $told) { Write-Host 'Waiting for 15 seconds without keyboard or mouse input (up to 4 minutes)...'; $told = $true }
        if ([DateTime]::Now -gt $deadline) { Write-Host 'SKIPPED the user never went idle; nothing was touched'; exit 2 }
        Start-Sleep -Milliseconds 500
    }
    # A locked screen takes neither input nor clipboard access.
    if (Get-Process LogonUI -ErrorAction SilentlyContinue) { Write-Host 'SKIPPED the screen is locked'; exit 2 }
    # Another program may hold the clipboard for a moment.
    foreach ($attempt in 1..20) {
        try { Save-Clipboard; break }
        catch { if ($attempt -eq 20) { Write-Host "SKIPPED the clipboard stayed busy: $_"; exit 2 }; Start-Sleep -Milliseconds 250 }
    }

    $script:root = Join-Path ([System.IO.Path]::GetTempPath()) "mocli-e2e-$PID"
    New-Item -ItemType Directory -Force "$root/appdata/mocli", "$root/files" | Out-Null
    $env:APPDATA = "$root/appdata"
    $env:MOCLI_DATA_DIR = "$root/data"
    $env:MOCLI_LOG = 'warn,mocli=debug'
}

# Stops mocli, restores the clipboard, reports and exits.
function Complete-Test {
    foreach ($vk in 0xA0, 0xA2, 0xA4, 0xA5, 0x5B) { [N]::Key($vk, $true) }
    Stop-Mocli
    if ($script:hasBackup) { $Forms::SetDataObject($script:backup, $true, 20, 50) } else { $Forms::Clear() }
    Write-Host "---- $script:failed failed"
    Write-Host '---- warnings and errors in log'
    Get-Log | Select-String ' WARN | ERROR ' | ForEach-Object { Write-Host $_ }
    Get-Content "$root/stderr.txt" -ErrorAction SilentlyContinue | Select-String 'panic' | ForEach-Object { Write-Host $_ }
    Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue
    exit [int]($script:failed -gt 0)
}

function Wait-Ui([int]$ms = 120) {
    $end = [DateTime]::Now.AddMilliseconds($ms)
    while ([DateTime]::Now -lt $end) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 5 }
}
function Start-Mocli([string]$config = '') {
    Set-Content "$root/appdata/mocli/config.toml" $config
    $script:app = Start-Process $Exe -PassThru -WindowStyle Hidden -RedirectStandardError "$root/stderr.txt"
    Wait-Ui 2500
    if ($script:app.HasExited) { throw "mocli exited: $(Get-Content "$root/stderr.txt" -Raw)" }
}
function Stop-Mocli {
    if ($script:app -and -not $script:app.HasExited) { Stop-Process -Id $script:app.Id; $script:app.WaitForExit() }
}

function Get-Log { Get-Content "$env:MOCLI_DATA_DIR\logs\mocli.*.log" -ErrorAction SilentlyContinue }
function Check([string]$name, [bool]$ok, $detail = '') {
    if ($ok) { Write-Host "PASS  $name" } else { $script:failed++; Write-Host "FAIL  $name  $detail" }
}

function Popup { [N]::VisibleWindow($script:app.Id) }
function Closed { (Popup) -eq [IntPtr]::Zero }
function Bounds { $r = New-Object N+RECT; [void][N]::GetWindowRect((Popup), [ref]$r); $r }

# Real input goes only to the test window or the popup.
function Assert-Safe {
    $front = [N]::GetForegroundWindow()
    if ($front -ne $script:target -and ($front -eq [IntPtr]::Zero -or $front -ne (Popup))) {
        throw "ABORT: another window is in front: '$([N]::Title($front))'"
    }
}
# Send-Chord 0x5B,0x56 = hold Win, press V, release in reverse order.
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
# Windows only lets the process that sent the latest input take the foreground,
# so tap an unassigned key first.
function Show-Front([IntPtr]$window) {
    [N]::Key(0xE8, $false); [N]::Key(0xE8, $true)
    [void][N]::SetForegroundWindow($window)
    Wait-Ui 300
}
# The default shortcut, Win+V.
function Open-Popup([int]$wait = 600) { Send-Chord @(0x5B, 0x56) $wait }
function Click-At([int]$x, [int]$y, [int]$wait = 300) {
    [void][N]::SetCursorPos($x, $y); Wait-Ui 50
    $under = [N]::RootAt($x, $y)
    if ($under -ne $script:target -and ($under -eq [IntPtr]::Zero -or $under -ne (Popup))) {
        throw "ABORT: another window covers the click point: '$([N]::Title($under))'"
    }
    [N]::Click(); Wait-Ui $wait
}

# This process stays the clipboard owner, so it is recorded as the source.
function Copy-Data($data) { $Forms::SetDataObject($data, $false, 20, 50); Wait-Ui 500 }
function New-Data([string]$text) {
    $data = [System.Windows.Forms.DataObject]::new()
    if ($text) { $data.SetText($text) }
    $data
}
