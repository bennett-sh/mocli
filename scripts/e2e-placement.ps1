<#
End-to-end check of where the popup opens and of both focus modes. Run after
`cargo build`:

    pwsh -STA -File scripts/e2e-placement.ps1

It opens its own test windows (WinForms, WPF, and Microsoft Edge with a
throwaway profile), types into them and overwrites the clipboard, so leave the
keyboard and mouse alone while it runs (about a minute). It waits for 15 idle
seconds before starting, and uses a temporary config and history. Quit any
running mocli first.
#>
param([string]$Exe = "$PSScriptRoot/../target/debug/mocli.exe")
. "$PSScriptRoot/e2e-lib.ps1"
Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase, WindowsFormsIntegration

$area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea

function Lookup {
    "$(Get-Log | Select-String 'caret lookup' | Select-Object -Last 1)" -replace '.*method="?([a-z ]+?)"?$', '$1'
}
function Show-Rect($r) { "popup $($r.L),$($r.T) - $($r.R),$($r.B)" }
function Test-Centered($r) {
    $x = [math]::Abs(($r.L + $r.R) / 2 - ($area.X + $area.Width / 2))
    $y = [math]::Abs(($r.T + $r.B) / 2 - ($area.Y + $area.Height / 2))
    $x -le 6 -and $y -le 6
}
# Whether the popup hangs just below the input at the given screen rectangle.
function Test-Below($r, $left, $top, $right, $bottom, $scale) {
    $r.T -ge ($top + ($bottom - $top) / 2) -and $r.T -le ($bottom + 24 * $scale) -and
        $r.L -ge ($left - 4) -and $r.L -le $right
}
function Close-Popup { Send-Chord @(0x1B) 400 }

function New-Form([string]$title, [int]$x, [int]$y) {
    $form = New-Object System.Windows.Forms.Form
    $form.Text = $title; $form.StartPosition = 'Manual'; $form.TopMost = $true
    $form.Location = [System.Drawing.Point]::new($x, $y); $form.Size = '700,130'
    $form
}
# Puts the form at a spot, clicks into its box and returns the box's rectangle.
function Use-Box([int]$x, [int]$y) {
    $form.Location = [System.Drawing.Point]::new($x, $y); Show-Front $form.Handle
    $field = $box.RectangleToScreen($box.ClientRectangle)
    $script:target = $form.Handle
    Click-At ($field.Right - 20) ($field.Top + $field.Height / 2)
    Assert-Safe
    $field
}

Start-Test
$form = $null; $plain = $null; $wpf = $null; $other = $null
$edgeRunning = $false
try {
    Start-Mocli
    Copy-Data (New-Data 'seed entry')

    # --- A classic Win32 text box ---
    $form = New-Form 'mocli e2e target' ($area.X + 150) ($area.Y + 80)
    $box = New-Object System.Windows.Forms.TextBox; $box.Dock = 'Top'; $form.Controls.Add($box)
    $form.Show(); Wait-Ui 300
    $scale = [N]::GetDpiForWindow($form.Handle) / 96.0
    $field = Use-Box ($area.X + 150) ($area.Y + 80)
    Send-Text 'abc'
    Open-Popup
    $first = Bounds
    Check 'Win32: popup opens below the caret' (Test-Below $first $field.Left $field.Top $field.Right $field.Bottom $scale) "$(Show-Rect $first), box $field, via $(Lookup)"
    Write-Host "INFO  Win32 text box found via $(Lookup)"
    Check 'Win32: the box keeps focus' ($box.Focused -and [N]::GetForegroundWindow() -eq $form.Handle)
    Close-Popup
    Send-Text ' and quite a bit more text'
    Open-Popup
    $second = Bounds
    Check 'popup follows the caret along the line' ($second.L -gt $first.L + 50) "$($first.L) -> $($second.L)"
    Close-Popup

    $box.Clear()
    $field = Use-Box ($area.X + 150) ($area.Bottom - 160)
    Open-Popup
    $r = Bounds
    Check 'near the bottom the popup flips above' ($r.B -le ($field.Top + 4) -and $r.T -ge $area.Top) "$(Show-Rect $r), box $field"
    Close-Popup

    # Enough text to put the caret near the right edge of the screen.
    $box.Text = 'mmmmmmmm ' * 12
    $field = Use-Box ($area.Right - 720) ($area.Y + 80)
    Open-Popup
    $r = Bounds
    Check 'near the right edge the popup stays on screen' ([math]::Abs($r.R - $area.Right) -le 2 -and $r.T -ge $field.Top) "$(Show-Rect $r), area ends at $($area.Right)"
    Close-Popup
    $box.Clear()

    # --- No text input at all ---
    $plain = New-Form 'mocli e2e buttons' ($area.X + 60) ($area.Y + 60)
    $button = New-Object System.Windows.Forms.Button; $button.Text = 'nothing to type into'; $button.Dock = 'Top'
    $plain.Controls.Add($button); $plain.Show(); Show-Front $plain.Handle
    $script:target = $plain.Handle
    $at = $button.PointToScreen([System.Drawing.Point]::new(20, 8)); Click-At $at.X $at.Y
    Open-Popup
    $r = Bounds
    Check 'without an input the popup is centered' (Test-Centered $r) "$(Show-Rect $r), via $(Lookup)"
    Close-Popup
    $plain.Close()

    # --- WPF has no Win32 caret, so this needs UI Automation ---
    $wpf = [System.Windows.Window]::new()
    $wpf.Title = 'mocli e2e wpf'; $wpf.Width = 560; $wpf.Height = 160; $wpf.Left = 120; $wpf.Top = 90; $wpf.Topmost = $true
    $text = [System.Windows.Controls.TextBox]::new()
    $text.Margin = [System.Windows.Thickness]::new(24); $text.Height = 28; $text.VerticalAlignment = 'Top'
    # Without this, a WinForms message loop never turns keys into WPF text.
    [System.Windows.Forms.Integration.ElementHost]::EnableModelessKeyboardInterop($wpf)
    $wpf.Content = $text; $wpf.Show(); Wait-Ui 700
    $script:target = [System.Windows.Interop.WindowInteropHelper]::new($wpf).Handle
    Show-Front $script:target
    $a = $text.PointToScreen([System.Windows.Point]::new(0, 0))
    $b = $text.PointToScreen([System.Windows.Point]::new($text.ActualWidth, $text.ActualHeight))
    Click-At ($b.X - 20) (($a.Y + $b.Y) / 2)
    Send-Text 'abc'
    Check 'WPF: typing reaches the box' ($text.Text -ceq 'abc') $text.Text
    Open-Popup 900
    $r = Bounds
    Check 'WPF: popup opens below the caret' (Test-Below $r $a.X $a.Y $b.X $b.Y $scale) "$(Show-Rect $r), box $a $b, via $(Lookup)"
    Check 'WPF: found through accessibility' ((Lookup) -ne 'none') (Lookup)
    Write-Host "INFO  WPF found via $(Lookup)"
    Send-Chord @(0x0D) 700
    Check 'WPF: Enter pastes into the box' ($text.Text -ceq 'abcseed entry') $text.Text
    $wpf.Close()

    # --- A Chromium browser ---
    $edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe", "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe" |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($edge) {
        @'
<!doctype html><title>mocli-edge:</title>
<textarea id="t" style="position:absolute;left:40px;top:40px;width:600px;height:40px;font-size:16px" autofocus></textarea>
<script>t.oninput = () => document.title = 'mocli-edge:' + t.value</script>
'@ | Set-Content "$root/page.html"
        $page = ([uri][System.IO.Path]::GetFullPath("$root/page.html")).AbsoluteUri
        $edgeRunning = $true
        Start-Process $edge "--user-data-dir=`"$root\edge`" --no-first-run --no-default-browser-check --disable-sync --window-position=80,80 --window-size=900,500 --app=$page"
        $window = [IntPtr]::Zero
        foreach ($i in 1..80) { Wait-Ui 250; $window = [N]::WindowTitled('mocli-edge:'); if ($window -ne [IntPtr]::Zero) { break } }
        Wait-Ui 1500
        Show-Front $window
        if ($window -ne [IntPtr]::Zero -and [N]::GetForegroundWindow() -eq $window) {
            $script:target = $window
            $w = New-Object N+RECT; [void][N]::GetWindowRect($window, [ref]$w)
            $s = [N]::GetDpiForWindow($window) / 96.0
            # The app window's title bar is about 33 logical pixels tall.
            $left = $w.L + 48 * $s; $right = $w.L + 648 * $s
            $top = $w.T + 73 * $s; $bottom = $w.T + 120 * $s
            Click-At ($left + 300 * $s) ($w.T + 95 * $s) 500
            Send-Text 'abc'
            Check 'Edge: typing reaches the page' ([N]::Title($window) -like 'mocli-edge:abc*') ([N]::Title($window))
            Open-Popup 900
            $r = Bounds
            Check 'Edge: popup opens below the caret' (Test-Below $r $left $top $right $bottom $s) "$(Show-Rect $r), area $left,$top - $right,$bottom, via $(Lookup)"
            Write-Host "INFO  Edge found via $(Lookup)"
            Check 'Edge: the page keeps focus' ([N]::GetForegroundWindow() -eq $window)
            Send-Chord @(0x0D) 900
            Check 'Edge: Enter pastes into the page' ([N]::Title($window) -like 'mocli-edge:abcseed entry*') ([N]::Title($window))
        } else {
            Check 'Edge: test window could be focused' $false "window=$window front='$([N]::Title([N]::GetForegroundWindow()))'"
        }
    } else {
        Write-Host 'INFO  Edge is not installed; browser check skipped'
    }

    # --- Fixed placements ---
    Stop-Mocli; Start-Mocli "[general]`nplacement = `"centered`""
    $field = Use-Box ($area.X + 150) ($area.Y + 80)
    Open-Popup
    $r = Bounds
    Check 'placement "centered" ignores the caret' (Test-Centered $r) (Show-Rect $r)
    Close-Popup

    Stop-Mocli; Start-Mocli "[general]`nplacement = `"cursor`""
    $field = Use-Box ($area.X + 150) ($area.Y + 80)
    $x = $area.X + 420; $y = $area.Y + 260
    [void][N]::SetCursorPos($x, $y); Wait-Ui 100
    Open-Popup
    $r = Bounds
    Check 'placement "cursor" opens at the pointer' ([math]::Abs($r.L - $x) -le 2 -and $r.T -gt $y -and $r.T -le $y + 24 * $scale) "$(Show-Rect $r), pointer $x,$y"
    Close-Popup

    # --- A popup that takes focus ---
    Stop-Mocli; Start-Mocli "[general]`nfocus_mode = `"activate`""
    $box.Clear()
    $field = Use-Box ($area.X + 150) ($area.Y + 80)
    Send-Text 'abc'
    # Until code sets the selection, WinForms selects everything whenever the
    # box gets focus back.
    $box.SelectionStart = 3; $box.SelectionLength = 0
    Open-Popup 900
    Check 'activate: popup takes focus' ([N]::GetForegroundWindow() -eq (Popup)) ([N]::Title([N]::GetForegroundWindow()))
    $r = Bounds
    Check 'activate: still placed at the caret' (Test-Below $r $field.Left $field.Top $field.Right $field.Bottom $scale) (Show-Rect $r)
    Send-Text 'seed'
    Check 'activate: typing stays in the popup' ($box.Text -ceq 'abc') $box.Text
    Send-Chord @(0x0D) 900
    Check 'activate: Enter returns focus and pastes' ($box.Text -ceq 'abcseed entry' -and [N]::GetForegroundWindow() -eq $form.Handle) $box.Text

    $box.Clear(); Open-Popup 900; Send-Chord @(0x1B) 600
    Check 'activate: Escape closes and returns focus' ((Closed) -and [N]::GetForegroundWindow() -eq $form.Handle -and $box.Focused)
    Send-Text 'x'
    Check 'activate: typing reaches the box again' ($box.Text -ceq 'x') $box.Text

    Open-Popup 900; Open-Popup 600
    Check 'activate: shortcut closes and returns focus' ((Closed) -and [N]::GetForegroundWindow() -eq $form.Handle)

    Open-Popup 900
    Click-At ($field.Right - 20) ($field.Top + $field.Height / 2) 600
    Check 'activate: clicking the app closes the popup' ((Closed) -and [N]::GetForegroundWindow() -eq $form.Handle)

    $other = New-Form 'mocli e2e other' ($area.X + 60) ($area.Bottom - 160)
    $other.Show(); Show-Front $other.Handle
    $field = Use-Box ($area.X + 150) ($area.Y + 80)
    Open-Popup 900
    # Switch to the other window with the mouse, as a person would.
    $script:target = $other.Handle
    $at = $other.PointToScreen([System.Drawing.Point]::new(40, 60)); Click-At $at.X $at.Y 800
    Check 'activate: switching away is not undone' ((Closed) -and [N]::GetForegroundWindow() -eq $other.Handle) ([N]::Title([N]::GetForegroundWindow()))

    $problems = @(Get-Log | Select-String ' WARN | ERROR ')
    Check 'nothing went wrong in the log' ($problems.Count -eq 0) "$($problems.Count) lines"
}
catch { $script:failed++; Write-Host "ERROR $_ $($_.ScriptStackTrace)" }
finally {
    if ($edgeRunning) {
        # Only the processes using the throwaway profile.
        Get-CimInstance Win32_Process -Filter "Name = 'msedge.exe'" |
            Where-Object CommandLine -Like "*mocli-e2e-$PID*" |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 800
    }
    foreach ($window in $other, $plain, $form) { if ($window) { $window.Dispose() } }
    Complete-Test
}
