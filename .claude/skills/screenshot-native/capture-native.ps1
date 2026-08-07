param(
    [Parameter(Mandatory=$true)]
    [ValidateSet("region", "window", "menu")]
    [string]$Mode,

    [Parameter(Mandatory=$true)]
    [string]$Out,

    [int]$Zoom = 4,

    # region mode: the screen rect to capture (negative X/Y valid on multi-monitor setups)
    [int]$X,
    [int]$Y,
    [int]$Width,
    [int]$Height,

    # window mode: process whose MainWindowHandle rect gets captured
    [string]$ProcessName,

    # menu mode: where to right-click to open the menu, and the rect to capture afterward
    [int]$ClickX,
    [int]$ClickY,
    [int]$CaptureX,
    [int]$CaptureY,
    [int]$CaptureWidth,
    [int]$CaptureHeight,
    [int]$DelayMs = 250,

    # menu mode, optional: a second left-click (e.g. a menu item) before capturing,
    # read its coordinates off a prior menu-mode screenshot, then re-run with this set
    [int]$ThenClickX = -1,
    [int]$ThenClickY = -1
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class NativeInput {
    [DllImport("user32.dll")]
    public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
    [DllImport("user32.dll")]
    public static extern bool SetCursorPos(int X, int Y);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
}
"@

# Real OS-level input, not SendKeys: SendKeys targets whatever window currently has
# keyboard focus, which is not necessarily the app under test (see SKILL.md gotcha).
function Send-Click([int]$px, [int]$py, [switch]$Right) {
    [NativeInput]::SetCursorPos($px, $py) | Out-Null
    Start-Sleep -Milliseconds 60
    if ($Right) {
        [NativeInput]::mouse_event(0x0008, 0, 0, 0, [UIntPtr]::Zero) # RIGHTDOWN
        [NativeInput]::mouse_event(0x0010, 0, 0, 0, [UIntPtr]::Zero) # RIGHTUP
    } else {
        [NativeInput]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero) # LEFTDOWN
        [NativeInput]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) # LEFTUP
    }
}

function Save-Region([int]$left, [int]$top, [int]$width, [int]$height, [int]$zoom, [string]$outPath) {
    if ($width -le 0 -or $height -le 0) {
        Write-Error "Capture width/height must be > 0 (got ${width}x${height})"
        exit 1
    }
    $raw = New-Object Drawing.Bitmap $width, $height
    $g = [Drawing.Graphics]::FromImage($raw)
    $g.CopyFromScreen($left, $top, 0, 0, $raw.Size)
    $g.Dispose()

    # Nearest-neighbour, not bicubic: a ~30px taskbar tile goes to mush under smoothing.
    $zoomed = New-Object Drawing.Bitmap ($width * $zoom), ($height * $zoom)
    $gz = [Drawing.Graphics]::FromImage($zoomed)
    $gz.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $gz.DrawImage($raw, 0, 0, $zoomed.Width, $zoomed.Height)
    $gz.Dispose()

    $outDir = Split-Path -Parent $outPath
    if ($outDir -and -not (Test-Path $outDir)) {
        New-Item -ItemType Directory -Force $outDir | Out-Null
    }
    $zoomed.Save($outPath, [Drawing.Imaging.ImageFormat]::Png)
    $raw.Dispose()
    $zoomed.Dispose()
}

switch ($Mode) {
    "region" {
        Save-Region $X $Y $Width $Height $Zoom $Out
    }
    "window" {
        $proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue |
            Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if (-not $proc) {
            Write-Error "No window with a MainWindowHandle for process '$ProcessName'. Borderless/tool windows (the strip, flyouts) often have none - use -Mode region with known coords instead."
            exit 1
        }
        $rect = New-Object NativeInput+RECT
        [NativeInput]::GetWindowRect($proc.MainWindowHandle, [ref]$rect) | Out-Null
        Save-Region $rect.Left $rect.Top ($rect.Right - $rect.Left) ($rect.Bottom - $rect.Top) $Zoom $Out
    }
    "menu" {
        # Everything from open to capture happens in this one process lifetime.
        # Splitting open and capture across two script calls lets the popup lose
        # activation and dismiss before the second call ever runs.
        Send-Click $ClickX $ClickY -Right
        Start-Sleep -Milliseconds $DelayMs
        if ($ThenClickX -ge 0 -and $ThenClickY -ge 0) {
            Send-Click $ThenClickX $ThenClickY
            Start-Sleep -Milliseconds $DelayMs
        }
        Save-Region $CaptureX $CaptureY $CaptureWidth $CaptureHeight $Zoom $Out
    }
}

Write-Host "Saved: $Out"
