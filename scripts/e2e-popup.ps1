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
. "$PSScriptRoot/e2e-lib.ps1"

function Save-Shot([string]$name, [IntPtr]$window = (Popup)) {
    Wait-Ui 400
    $r = New-Object N+RECT; [void][N]::GetWindowRect($window, [ref]$r)
    $bmp = New-Object System.Drawing.Bitmap (($r.R - $r.L), ($r.B - $r.T))
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size); $bmp.Save("$Shots/$name.png"); $g.Dispose(); $bmp.Dispose()
}
# Clicks a point given in the popup's logical pixels (760 wide).
function Click-Popup([double]$x, [double]$y, [int]$wait = 300) {
    $r = Bounds; $scale = ($r.R - $r.L) / 760.0
    Click-At ([int]($r.L + $x * $scale)) ([int]($r.T + $y * $scale)) $wait
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

Start-Test
New-Item -ItemType Directory -Force $Shots | Out-Null
$form = $null; $other = $null; $stripes = $null
try {
    Start-Mocli
    Check 'app is running and ready' ([bool](Get-Log | Select-String 'ready'))
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
    $form.Show(); Show-Front $form.Handle
    $script:target = $form.Handle
    $p = $box.PointToScreen([System.Drawing.Point]::new(100, 8)); Click-At $p.X $p.Y
    Assert-Safe
    Send-Text 'abc'
    Check 'baseline typing reaches the text box' ($box.Text -ceq 'abc') $box.Text

    Open-Popup
    Check 'Win+V shows the popup' (-not (Closed))
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
        # The link is not the newest entry, so only the filter brings it up.
        Send-Chord @(0x0D)
    }
    Check 'dropdown search selects a type' ($got -ceq 'https://example.com/cherry') $got
    $got = Use-Popup { Click-Popup 660 30 500; Send-Chord @(0x1B) 500; Send-Text 'delta'; Send-Chord @(0x0D) }
    Check 'closing the dropdown returns to the search' ($got -ceq 'delta dates') $got

    $got = Use-Popup { Send-Chord @(0xA2, 0x4B) 600; Save-Shot 'actions'; Send-Chord @(0x1B) 400; Send-Text 'delta'; Send-Chord @(0x0D) }
    Check 'Escape closes the actions panel, not the popup' ($got -ceq 'delta dates') $got

    # Ctrl+, opens the settings window, which takes focus like any window.
    Open-Popup; Send-Chord @(0xA2, 0xBC) 2500
    $window = [N]::GetForegroundWindow()
    Check 'Ctrl+, opens the settings window' ([N]::Title($window) -ceq 'mocli Settings') ([N]::Title($window))
    if ([N]::Title($window) -ceq 'mocli Settings') {
        $script:target = $window
        Save-Shot 'settings' $window
        Send-Chord @(0xA4, 0x73) 800
        $script:target = $form.Handle
    }
    Show-Front $form.Handle; Click-At $p.X $p.Y
    Check 'closing the settings keeps mocli running' (-not $app.HasExited)

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
    Check 'no Start menu after Win+V' ($box.Text -ceq 'e') $box.Text

    # Type immediately, before the popup has had time to appear.
    $box.Clear(); Wait-Ui 100
    Open-Popup 0; Send-Text 'delta'; Send-Chord @(0x0D) 600
    Check 'type-ahead is not lost' ($box.Text -ceq 'delta dates') $box.Text

    $box.Clear(); Open-Popup
    Click-At $p.X $p.Y
    Check 'click outside closes' (Closed)
    Send-Text 'f'
    Check 'keyboard released after outside click' ($box.Text -ceq 'f') $box.Text

    Open-Popup
    $other = New-Object System.Windows.Forms.Form; $other.TopMost = $true; $other.StartPosition = 'Manual'; $other.Location = '20,200'
    $other.Show(); Show-Front $other.Handle; Wait-Ui 300
    Check 'foreground change closes' (Closed)
    # GPUI asks a closed window for its bounds once more and logs that it is gone.
    $problems = @(Get-Log | Select-String ' WARN | ERROR ' |
        Where-Object { $_ -notmatch 'Invalid window handle' })
    Check 'nothing went wrong in the log' ($problems.Count -eq 0) "$($problems.Count) lines"
}
catch { $script:failed++; Write-Host "ERROR $_ $($_.ScriptStackTrace)" }
finally {
    foreach ($window in $other, $form, $stripes) { if ($window) { $window.Dispose() } }
    Write-Host "---- screenshots in $Shots"
    Complete-Test
}
