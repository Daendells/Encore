# Usage: powershell -File scripts\measure-resources.ps1 -Seconds 300
# Samples larik.exe and all its child processes (WebView2) every 5 s while a song is playing.
param([int]$Seconds = 300)

$interval = 5
$cores = [Environment]::ProcessorCount

function Get-Tree {
    $root = Get-Process -Name larik -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $root) { return @() }
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
    $ids = [System.Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($root.Id)
    do {
        $before = $ids.Count
        foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId)) { [void]$ids.Add([int]$p.ProcessId) } }
    } while ($ids.Count -gt $before)
    Get-Process -Id ([int[]]$ids) -ErrorAction SilentlyContinue
}

if (-not (Get-Tree)) { Write-Error "larik.exe is not running"; exit 1 }

$samples = @()
$remotes = @{}
$prev = $null
$end = (Get-Date).AddSeconds($Seconds)
while ((Get-Date) -lt $end) {
    $procs = @(Get-Tree)
    $now = Get-Date
    $cpu = 0.0
    foreach ($p in $procs) { try { $cpu += $p.TotalProcessorTime.TotalSeconds } catch {} }
    if ($prev -and $procs.Count) {
        $wall = ($now - $prev.Time).TotalSeconds
        $delta = [Math]::Max(0.0, $cpu - $prev.Cpu)
        $samples += [pscustomobject]@{
            CpuOneCore  = 100.0 * $delta / $wall
            CpuAllCores = 100.0 * $delta / $wall / $cores
            WorkingSet  = ($procs | Measure-Object WorkingSet64 -Sum).Sum / 1MB
            Private     = ($procs | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB
        }
    }
    $prev = @{ Time = $now; Cpu = $cpu }
    $ids = @($procs | ForEach-Object { $_.Id })
    Get-NetTCPConnection -ErrorAction SilentlyContinue |
        Where-Object { $ids -contains $_.OwningProcess -and $_.RemoteAddress -notin @('0.0.0.0', '::', '127.0.0.1', '::1') } |
        ForEach-Object { $remotes["$($_.RemoteAddress):$($_.RemotePort)"] = $_.RemoteAddress }
    Start-Sleep -Seconds $interval
}

if (-not $samples) { Write-Error "no samples (larik.exe exited?)"; exit 1 }
function Row($label, $values, $fmt) {
    $m = $values | Measure-Object -Average -Maximum
    "{0,-46} avg {1,9}   max {2,9}" -f $label, ($m.Average.ToString($fmt)), ($m.Maximum.ToString($fmt))
}
"Larik resource run: $($samples.Count) samples over $Seconds s, every $interval s, $cores logical cores"
Row "CPU, % of all logical cores (Task Manager)" $samples.CpuAllCores "0.00"
Row "CPU, % of one core" $samples.CpuOneCore "0.00"
Row "RAM, working set sum of tree (MB)" $samples.WorkingSet "0.0"
Row "RAM, private bytes sum of tree (MB)" $samples.Private "0.0"

"`nRemote TCP endpoints of the process tree seen during the run:"
$lr = @()
try { $lr = [System.Net.Dns]::GetHostAddresses("lrclib.net") | ForEach-Object { $_.ToString() } } catch {}
"lrclib.net resolves to: $($lr -join ', ')"
if (-not $remotes.Count) { "(none)" }
foreach ($k in ($remotes.Keys | Sort-Object)) {
    $addr = $remotes[$k]
    $name = try { [System.Net.Dns]::GetHostEntry($addr).HostName } catch { "(no reverse name)" }
    $tag = if ($lr -contains $addr) { "LRCLIB" } else { "NOT lrclib.net" }
    "{0,-26} {1,-40} {2}" -f $k, $name, $tag
}
