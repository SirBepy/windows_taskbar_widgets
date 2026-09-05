<#
.SYNOPSIS
  Win32 top-level window introspection: what windows exist, where they sit, and what a click hits.

.DESCRIPTION
  One shared P/Invoke block behind five modes, replacing the ad-hoc `Add-Type` blocks this repo
  kept re-deriving. See SKILL.md for what this probe CANNOT see.

.EXAMPLE
  .\window-probe.ps1 -Pid 1234
  .\window-probe.ps1 -At 500,1400
  .\window-probe.ps1 -Region 0,1392,468,1440
  .\window-probe.ps1 -Topmost
  .\window-probe.ps1 -WatchClicks 20
#>
[CmdletBinding(DefaultParameterSetName = 'ProcessId')]
param(
  [Parameter(ParameterSetName = 'ProcessId', Mandatory)]
  [Alias('Pid')]
  [int]$ProcessId,

  [Parameter(ParameterSetName = 'At', Mandatory)]
  [string]$At,

  [Parameter(ParameterSetName = 'Region', Mandatory)]
  [string]$Region,

  [Parameter(ParameterSetName = 'Topmost', Mandatory)]
  [switch]$Topmost,

  [Parameter(ParameterSetName = 'WatchClicks', Mandatory)]
  [int]$WatchClicks
)

$ErrorActionPreference = 'Stop'

if (-not ('WinProbe.Native' -as [type])) {
  Add-Type -Namespace 'WinProbe' -Name 'Native' -MemberDefinition @'
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
[StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }

public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);

[DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
[DllImport("user32.dll")] public static extern int GetWindowLongW(IntPtr hWnd, int idx);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hWnd, StringBuilder s, int max);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int max);
[DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
[DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
[DllImport("user32.dll")] public static extern short GetAsyncKeyState(int vKey);
[DllImport("user32.dll")] public static extern int GetWindowThreadProcessId(IntPtr hWnd, out int pid);
[DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hWnd, uint flags);
'@ -UsingNamespace 'System.Text'
}

# Coords arrive as one string, never int[]: `powershell -File script.ps1 -At 5,6` hands the
# whole `5,6` to a single parameter, so an [int[]] binding fails before the script runs.
function ConvertTo-IntList {
  param([string]$Text, [int]$Count, [string]$Name, [string]$Example)
  $parts = $Text -split '[,\s]+' | Where-Object { $_ -ne '' }
  if (@($parts).Count -ne $Count) { throw ("ERROR: -{0} needs {1} ints, e.g. -{0} {2}" -f $Name, $Count, $Example) }
  $out = New-Object System.Collections.Generic.List[int]
  foreach ($p in $parts) {
    $n = 0
    if (-not [int]::TryParse($p, [ref]$n)) { throw ("ERROR: -{0}: '{1}' is not an integer" -f $Name, $p) }
    $out.Add($n)
  }
  $out
}

$GWL_EXSTYLE   = -20
$WS_EX_TOPMOST = 0x00000008
$WS_EX_LAYERED = 0x00080000
$WS_EX_NOACTIVATE = 0x08000000
$GA_ROOT = 2

function Get-WindowInfo {
  param([IntPtr]$Handle)

  $rect = New-Object WinProbe.Native+RECT
  $null = [WinProbe.Native]::GetWindowRect($Handle, [ref]$rect)

  $sb = New-Object System.Text.StringBuilder 512
  $null = [WinProbe.Native]::GetWindowTextW($Handle, $sb, $sb.Capacity)
  $title = $sb.ToString()

  $sb2 = New-Object System.Text.StringBuilder 512
  $null = [WinProbe.Native]::GetClassNameW($Handle, $sb2, $sb2.Capacity)

  $ownerPid = 0
  $null = [WinProbe.Native]::GetWindowThreadProcessId($Handle, [ref]$ownerPid)

  $ex = [WinProbe.Native]::GetWindowLongW($Handle, $GWL_EXSTYLE)

  $flags = New-Object System.Collections.Generic.List[string]
  if ($ex -band $WS_EX_TOPMOST)    { $flags.Add('TOPMOST') }
  if ($ex -band $WS_EX_LAYERED)    { $flags.Add('LAYERED') }
  if ($ex -band $WS_EX_NOACTIVATE) { $flags.Add('NOACTIVATE') }

  $procName = '?'
  try { $procName = (Get-Process -Id $ownerPid -ErrorAction Stop).ProcessName } catch { $procName = '?' }

  [pscustomobject]@{
    Handle    = $Handle
    Pid       = $ownerPid
    Process   = $procName
    Title     = $title
    Class     = $sb2.ToString()
    Visible   = [WinProbe.Native]::IsWindowVisible($Handle)
    Left      = $rect.Left
    Top       = $rect.Top
    Right     = $rect.Right
    Bottom    = $rect.Bottom
    Width     = $rect.Right - $rect.Left
    Height    = $rect.Bottom - $rect.Top
    ExStyle   = ('0x{0:X8}' -f $ex)
    Flags     = ($flags -join ',')
  }
}

# Z-ordered: EnumWindows walks top-level windows front to back, and that order is the
# only reason a hit list reads as a stack rather than a set.
function Get-AllWindows {
  $handles = New-Object System.Collections.Generic.List[IntPtr]
  $cb = [WinProbe.Native+EnumProc] {
    param([IntPtr]$h, [IntPtr]$l)
    $handles.Add($h)
    return $true
  }
  $null = [WinProbe.Native]::EnumWindows($cb, [IntPtr]::Zero)
  foreach ($h in $handles) { Get-WindowInfo -Handle $h }
}

function Get-DescendantPids {
  param([int]$Root)
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
  $found = New-Object System.Collections.Generic.List[int]
  $found.Add($Root)
  $queue = New-Object System.Collections.Generic.Queue[int]
  $queue.Enqueue($Root)
  while ($queue.Count -gt 0) {
    $cur = $queue.Dequeue()
    foreach ($p in $all) {
      if ($p.ParentProcessId -eq $cur -and -not $found.Contains([int]$p.ProcessId)) {
        $found.Add([int]$p.ProcessId)
        $queue.Enqueue([int]$p.ProcessId)
      }
    }
  }
  $found
}

function Format-Rows {
  param($Rows)
  if (-not $Rows -or @($Rows).Count -eq 0) {
    Write-Output '(no windows matched)'
    return
  }
  # Out-String -Width, not the host's: run non-interactively the console is 80 cols and
  # Format-Table silently truncates ExStyle/Flags, the two columns this probe exists for.
  $Rows |
    Format-Table -AutoSize Process, Pid, Handle, Title, Class, Visible, Left, Top, Width, Height, ExStyle, Flags |
    Out-String -Width 300 |
    Write-Output
}

switch ($PSCmdlet.ParameterSetName) {

  'ProcessId' {
    $pids = Get-DescendantPids -Root $ProcessId
    Write-Output ('Process tree: ' + ($pids -join ', '))
    # Hidden windows are never filtered here: a parked window reading vis=False at
    # -32000,-32000 is the exact thing this mode was built to confirm.
    $rows = Get-AllWindows | Where-Object { $pids -contains $_.Pid }
    Format-Rows -Rows $rows
  }

  'At' {
    $xy = ConvertTo-IntList -Text $At -Count 2 -Name 'At' -Example '500,1400'
    $pt = New-Object WinProbe.Native+POINT
    $pt.X = $xy[0]
    $pt.Y = $xy[1]
    $hit = [WinProbe.Native]::WindowFromPoint($pt)
    $root = [WinProbe.Native]::GetAncestor($hit, $GA_ROOT)
    Write-Output ('WindowFromPoint({0},{1}) -> handle {2} (root {3})' -f $xy[0], $xy[1], $hit, $root)
    Write-Output 'WARNING: a hidden WebView2 window still swallows clicks here and does NOT appear as the hit. See SKILL.md.'
    Write-Output ''
    Write-Output 'Every window whose rect contains the point, front to back (hidden included):'
    $rows = Get-AllWindows | Where-Object {
      $_.Left -le $xy[0] -and $xy[0] -lt $_.Right -and $_.Top -le $xy[1] -and $xy[1] -lt $_.Bottom
    }
    Format-Rows -Rows $rows
  }

  'Region' {
    $box = ConvertTo-IntList -Text $Region -Count 4 -Name 'Region' -Example '0,1392,468,1440'
    $l = $box[0]; $t = $box[1]; $r = $box[2]; $b = $box[3]
    $rows = Get-AllWindows | Where-Object {
      $_.Visible -and $_.Left -lt $r -and $_.Right -gt $l -and $_.Top -lt $b -and $_.Bottom -gt $t
    }
    Write-Output ('Visible windows overlapping {0},{1},{2},{3}:' -f $l, $t, $r, $b)
    Format-Rows -Rows $rows
  }

  'Topmost' {
    $rows = Get-AllWindows | Where-Object { $_.Visible -and $_.Flags -match 'TOPMOST' }
    Write-Output 'Visible WS_EX_TOPMOST windows, desktop-wide:'
    Format-Rows -Rows $rows
  }

  'WatchClicks' {
    $VK_LBUTTON = 0x01
    $deadline = (Get-Date).AddSeconds($WatchClicks)
    Write-Output ('Watching left-clicks for {0}s. Ctrl+C to stop early.' -f $WatchClicks)
    $down = $false
    while ((Get-Date) -lt $deadline) {
      $state = [WinProbe.Native]::GetAsyncKeyState($VK_LBUTTON)
      $isDown = ($state -band 0x8000) -ne 0
      if ($isDown -and -not $down) {
        $pt = New-Object WinProbe.Native+POINT
        $null = [WinProbe.Native]::GetCursorPos([ref]$pt)
        $hit = [WinProbe.Native]::WindowFromPoint($pt)
        $info = Get-WindowInfo -Handle $hit
        $line = 'click {0},{1} -> {2} pid={3} "{4}" class={5} rect={6},{7} {8}x{9}' -f `
          $pt.X, $pt.Y, $info.Handle, $info.Pid, $info.Title, $info.Class,
          $info.Left, $info.Top, $info.Width, $info.Height
        Write-Output $line
      }
      $down = $isDown
      Start-Sleep -Milliseconds 30
    }
    Write-Output 'Watch ended.'
  }
}
