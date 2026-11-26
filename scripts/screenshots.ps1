<#
Takes the screenshots in assets/ (for the readme and the releases).

    cargo build --release
    pwsh -STA -File scripts/screenshots.ps1 [-Background my-picture.jpg]

It covers the desktop and taskbar with a background picture (a generated
gradient unless you pass one), fills a temporary history with made-up entries,
opens the popup with Win+V and saves the popup in several states, in dark and
light, plus the settings window. Your own config and history are not touched.

It types and clicks for real, so leave the keyboard and mouse alone while it
runs (about a minute). It waits for 15 idle seconds first, and puts the taskbar
and clipboard back afterwards. Quit any running mocli first. If the taskbar
stays hidden after an abort, run it again or restart Explorer.

The settings pages are reached by clicking the sidebar at fixed positions
(`$SidebarX`, `$SidebarY`, `$SidebarStep`); adjust them if a page comes out wrong.
#>
param(
    [string]$Exe = "$PSScriptRoot/../target/release/mocli.exe",
    [string]$Out = "$PSScriptRoot/../assets",
    [string]$Background = '',
    # Extra pixels around the popup, so the backdrop shows.
    [int]$Margin = 56
)
. "$PSScriptRoot/e2e-lib.ps1"

# Position of the settings sidebar in the window's logical pixels (920x640).
$SidebarX = 100; $SidebarY = 100; $SidebarStep = 38
$SettingsPages = 'general', 'appearance', 'history', 'privacy', 'shortcuts', 'advanced', 'about'

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Taskbar {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string c, string t);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    public static void Show(bool show) {
        foreach (var c in new[] { "Shell_TrayWnd", "Shell_SecondaryTrayWnd" }) {
            var h = FindWindow(c, null);
            if (h != IntPtr.Zero) ShowWindow(h, show ? 5 : 0);
        }
    }
}
'@

if (-not (Test-Path $Exe)) { Write-Host "SKIPPED $Exe is missing; run cargo build --release"; exit 2 }
$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$Out = (Resolve-Path $Out).Path

function New-Backdrop([int]$width, [int]$height) {
    if ($Background) { return [System.Drawing.Image]::FromFile((Resolve-Path $Background).Path) }
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.SmoothingMode = 'AntiAlias'
    $rect = New-Object System.Drawing.Rectangle 0, 0, $width, $height
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect,
        ([System.Drawing.Color]::FromArgb(255, 32, 18, 74)), ([System.Drawing.Color]::FromArgb(255, 255, 112, 64)), 35
    $g.FillRectangle($brush, $rect)
    # Soft blobs give the acrylic something to blur.
    $blobs = @(
        @(0.15, 0.25, 0.45, 255, 80, 200), @(0.8, 0.2, 0.35, 70, 160, 255),
        @(0.65, 0.85, 0.5, 255, 190, 90), @(0.1, 0.9, 0.3, 190, 60, 160))
    foreach ($b in $blobs) {
        $d = [int]($height * $b[2]); $x = [int]($width * $b[0] - $d / 2); $y = [int]($height * $b[1] - $d / 2)
        $path = New-Object System.Drawing.Drawing2D.GraphicsPath
        $path.AddEllipse($x, $y, $d, $d)
        $fill = New-Object System.Drawing.Drawing2D.PathGradientBrush $path
        $fill.CenterColor = [System.Drawing.Color]::FromArgb(170, $b[3], $b[4], $b[5])
        $fill.SurroundColors = @([System.Drawing.Color]::FromArgb(0, $b[3], $b[4], $b[5]))
        $g.FillPath($fill, $path)
    }
    $g.Dispose()
    $bitmap
}

# Saves the screen around a window, `$Margin` pixels beyond its edges.
function Save-Shot([string]$name, [IntPtr]$window = (Popup), [int]$margin = $Margin) {
    Wait-Ui 500
    $r = New-Object N+RECT; [void][N]::GetWindowRect($window, [ref]$r)
    $screen = [System.Windows.Forms.Screen]::FromHandle($window).Bounds
    $left = [Math]::Max($screen.Left, $r.L - $margin); $top = [Math]::Max($screen.Top, $r.T - $margin)
    $right = [Math]::Min($screen.Right, $r.R + $margin); $bottom = [Math]::Min($screen.Bottom, $r.B + $margin)
    $bmp = New-Object System.Drawing.Bitmap ($right - $left), ($bottom - $top)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($left, $top, 0, 0, $bmp.Size)
    $bmp.Save("$Out/$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "saved $name.png"
}

function Set-Config([string]$theme) {
    Set-Content "$root/appdata/mocli/config.toml" "[appearance]`ntheme = `"$theme`"`nanimations = false`n"
    # The app polls the file once a second.
    Wait-Ui 2000
}
function Close-Popup { foreach ($i in 1..3) { if (Closed) { return }; Send-Chord @(0x1B) 300 } }
function Down([int]$times = 1) { foreach ($i in 1..$times) { Send-Chord @(0x28) 120 } }

function Add-Entries {
    # Oldest first, so the newest ends up at the top of the list.
    Copy-Data (New-Data 'hello@example.com')
    Copy-Data (New-Data '#ff7a1a')
    Copy-Data (New-Data "fn fibonacci(n: u32) -> u64 {`n    match n {`n        0 => 0,`n        1 => 1,`n        _ => fibonacci(n - 1) + fibonacci(n - 2),`n    }`n}")
    Copy-Data (New-Data 'https://github.com/bennett-sh/mocli')

    $bitmap = New-Object System.Drawing.Bitmap 640, 400
    $g = [System.Drawing.Graphics]::FromImage($bitmap)
    $g.SmoothingMode = 'AntiAlias'
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.Rectangle 0, 0, 640, 400),
        ([System.Drawing.Color]::FromArgb(255, 20, 90, 200)), ([System.Drawing.Color]::FromArgb(255, 120, 220, 190)), 60
    $g.FillRectangle($brush, 0, 0, 640, 400)
    $g.FillEllipse([System.Drawing.Brushes]::Gold, 420, 50, 120, 120)
    $g.FillPolygon([System.Drawing.Brushes]::White, @(
        [System.Drawing.Point]::new(0, 400), [System.Drawing.Point]::new(180, 170),
        [System.Drawing.Point]::new(300, 300), [System.Drawing.Point]::new(420, 210), [System.Drawing.Point]::new(640, 400)))
    $g.Dispose()
    $data = New-Data ''; $data.SetImage($bitmap); Copy-Data $data

    $names = 'Quarterly report.pdf', 'budget.xlsx'
    $list = [System.Collections.Specialized.StringCollection]::new()
    foreach ($n in $names) {
        $path = Join-Path "$root/files" $n
        'x' * 2048 | Set-Content $path -NoNewline
        [void]$list.Add([System.IO.Path]::GetFullPath($path))
    }
    $data = New-Data ''; $data.SetFileDropList($list); Copy-Data $data
    Copy-Data (New-Data "Meeting notes`n- ship the 1.0 release on Friday`n- ask Sam about the icon`n- book the venue")
}

Start-Test
$form = $null
try {
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $form = New-Object System.Windows.Forms.Form
    $form.FormBorderStyle = 'None'; $form.StartPosition = 'Manual'; $form.ShowInTaskbar = $false
    $form.Bounds = $screen
    $form.BackgroundImage = New-Backdrop $screen.Width $screen.Height
    $form.BackgroundImageLayout = 'Zoom'
    $form.BackColor = [System.Drawing.Color]::Black
    $form.Show()
    [Taskbar]::Show($false)
    Show-Front $form.Handle
    $script:target = $form.Handle
    [void][N]::SetCursorPos($screen.Right - 1, $screen.Bottom - 1)

    Set-Config 'dark'
    Start-Mocli (Get-Content "$root/appdata/mocli/config.toml" -Raw)
    Show-Front $form.Handle
    Add-Entries
    Show-Front $form.Handle

    # Dark: the list, each kind of preview, the actions, filter and search.
    Open-Popup
    Save-Shot 'popup-dark'
    $kinds = 'files', 'image', 'link', 'code', 'color', 'email'
    foreach ($kind in $kinds) {
        Down
        Save-Shot "preview-$kind"
    }
    Send-Chord @(0xA2, 0x4B) 500
    Save-Shot 'actions'
    Send-Chord @(0x1B) 400
    Send-Chord @(0x09) 400
    Save-Shot 'type-filter'
    Send-Chord @(0x10, 0x09) 300
    Send-Text 'fib'
    Save-Shot 'search'
    Close-Popup

    # Light.
    Set-Config 'light'
    Show-Front $form.Handle
    Open-Popup
    Save-Shot 'popup-light'
    Down 2
    Save-Shot 'preview-image-light'
    Close-Popup

    # The settings window, a real window, in both themes.
    Open-Popup
    Send-Chord @(0xA2, 0xBC) 1200
    $settings = [N]::WindowTitled('mocli Settings')
    Check 'settings window opens' ($settings -ne [IntPtr]::Zero)
    if ($settings -ne [IntPtr]::Zero) {
        $script:target = $settings
        $r = New-Object N+RECT; [void][N]::GetWindowRect($settings, [ref]$r)
        $scale = ($r.R - $r.L) / 920.0
        for ($i = 0; $i -lt $SettingsPages.Count; $i++) {
            Click-At ([int]($r.L + $SidebarX * $scale)) ([int]($r.T + ($SidebarY + $i * $SidebarStep) * $scale)) 500
            Save-Shot "settings-$($SettingsPages[$i])" $settings 24
        }
        $script:target = $form.Handle
    }
}
finally {
    [Taskbar]::Show($true)
    if ($form) { $form.Close() }
}
Complete-Test
