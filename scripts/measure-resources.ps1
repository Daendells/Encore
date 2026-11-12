# Usage: powershell -File scripts\measure-resources.ps1 -Seconds 300
# Samples larik.exe and all its child processes (WebView2) while a song is playing: memory and CPU every 5 s,
# TCP connections every 200 ms. The RAM figure to quote is private bytes (memory the app has committed);
# the working set is shown too, but it counts DLL pages shared with other programs.
param([int]$Seconds = 300)

$interval = 5
$pollMs = 200
$cores = [Environment]::ProcessorCount

# larik.exe and its descendants, each with a kind: larik, browser, renderer, gpu, network, storage, other.
function Get-Tree {
    $root = Get-Process -Name larik -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $root) { return @() }
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine
    $ids = [System.Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($root.Id)
    do {
        $before = $ids.Count
        foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId)) { [void]$ids.Add([int]$p.ProcessId) } }
    } while ($ids.Count -gt $before)
    foreach ($p in $all) {
        if (-not $ids.Contains([int]$p.ProcessId)) { continue }
        $c = [string]$p.CommandLine
        $kind = if ($p.Name -eq 'larik.exe') { 'larik.exe' }
        elseif ($c -match '--type=renderer') { 'renderer' }
        elseif ($c -match '--type=gpu-process') { 'gpu' }
        elseif ($c -match 'NetworkService') { 'network' }
        elseif ($c -match 'StorageService') { 'storage' }
        elseif ($c -match '--type=crashpad') { 'crashpad' }
        elseif ($c -match '--type=') { 'other' }
        else { 'browser' }
        [pscustomobject]@{ Id = [int]$p.ProcessId; Kind = $kind }
    }
}

if (-not (Get-Tree)) { Write-Error "larik.exe is not running"; exit 1 }

$samples = @()
$perKind = @{}   # kind -> list of { Private, WorkingSet } sums, one per sample
$remotes = @{}   # "address:port" -> { Seconds, Kind } at first sight
$prev = @{}      # PID -> total CPU seconds at the last sample
$prevTime = $null
$lr = @{}
function Add-Lrclib { try { [System.Net.Dns]::GetHostAddresses("lrclib.net") | ForEach-Object { $lr[$_.ToString()] = $true } } catch {} }
Add-Lrclib
$start = Get-Date
$end = $start.AddSeconds($Seconds)
$nextSample = $start
while ((Get-Date) -lt $end) {
    $tree = @(Get-Tree)
    $now = Get-Date
    if ($now -ge $nextSample -and $tree.Count) {
        $nextSample = $now.AddSeconds($interval)
        # CPU delta per PID: a process that appears or exits during the run adds only its own time
        $delta = 0.0
        $cur = @{}
        $sums = @{}
        $ws = 0.0; $priv = 0.0
        foreach ($t in $tree) {
            $p = Get-Process -Id $t.Id -ErrorAction SilentlyContinue
            if (-not $p) { continue }
            try { $cpu = $p.TotalProcessorTime.TotalSeconds } catch { continue }
            $cur[$p.Id] = $cpu
            if ($prev.ContainsKey($p.Id)) { $delta += [Math]::Max(0.0, $cpu - $prev[$p.Id]) }
            if (-not $sums.ContainsKey($t.Kind)) { $sums[$t.Kind] = @{ Private = 0.0; WorkingSet = 0.0 } }
            $sums[$t.Kind].Private += $p.PrivateMemorySize64 / 1MB
            $sums[$t.Kind].WorkingSet += $p.WorkingSet64 / 1MB
            $ws += $p.WorkingSet64 / 1MB
            $priv += $p.PrivateMemorySize64 / 1MB
        }
        if ($prevTime) {
            $wall = ($now - $prevTime).TotalSeconds
            $samples += [pscustomobject]@{
                CpuOneCore  = 100.0 * $delta / $wall
                CpuAllCores = 100.0 * $delta / $wall / $cores
                WorkingSet  = $ws
                Private     = $priv
            }
            foreach ($k in $sums.Keys) {
                if (-not $perKind.ContainsKey($k)) { $perKind[$k] = @() }
                $perKind[$k] += [pscustomobject]$sums[$k]
            }
        }
        $prev = $cur
        $prevTime = $now
    }
    $kinds = @{}
    foreach ($t in $tree) { $kinds[$t.Id] = $t.Kind }
    Get-NetTCPConnection -ErrorAction SilentlyContinue |
        Where-Object { $kinds.ContainsKey([int]$_.OwningProcess) -and $_.RemoteAddress -notin @('0.0.0.0', '::', '127.0.0.1', '::1') } |
        ForEach-Object {
            $key = "$($_.RemoteAddress):$($_.RemotePort)"
            if (-not $remotes.ContainsKey($key)) {
                $remotes[$key] = @{ Address = $_.RemoteAddress; Seconds = [int]($now - $start).TotalSeconds; Kind = $kinds[[int]$_.OwningProcess] }
            }
        }
    Start-Sleep -Milliseconds $pollMs
}

if (-not $samples) { Write-Error "no samples (larik.exe exited?)"; exit 1 }
function Row($label, $values, $fmt) {
    $m = $values | Measure-Object -Average -Maximum
    "{0,-46} avg {1,9}   max {2,9}" -f $label, ($m.Average.ToString($fmt)), ($m.Maximum.ToString($fmt))
}
"Larik resource run: $($samples.Count) samples over $Seconds s, every $interval s, $cores logical cores"
Row "CPU, % of all logical cores (Task Manager)" $samples.CpuAllCores "0.00"
Row "CPU, % of one core" $samples.CpuOneCore "0.00"
Row "RAM, private bytes sum of tree (MiB)" $samples.Private "0.0"
Row "RAM, working set sum of tree (MiB)" $samples.WorkingSet "0.0"

"`nAverage per kind of process (MiB):"
"{0,-12} {1,10} {2,12}" -f "kind", "private", "working set"
foreach ($k in ($perKind.Keys | Sort-Object)) {
    "{0,-12} {1,10:N1} {2,12:N1}" -f $k, ($perKind[$k] | Measure-Object Private -Average).Average, ($perKind[$k] | Measure-Object WorkingSet -Average).Average
}

"`nRemote TCP endpoints of the process tree seen during the run:"
Add-Lrclib
$lrList = @($lr.Keys)
"Method: TCP connections only, sampled every $pollMs ms. Very short connections and UDP/QUIC are not seen."
"lrclib.net resolved at the start and end of the run to: $($lrList -join ', ')"
if (-not $remotes.Count) { "(none)" }
foreach ($k in ($remotes.Keys | Sort-Object { $remotes[$_].Seconds })) {
    $r = $remotes[$k]
    $name = try { [System.Net.Dns]::GetHostEntry($r.Address).HostName } catch { "(no reverse name)" }
    $tag = if ($lrList -contains $r.Address) { "LRCLIB" } else { "NOT lrclib.net" }
    "{0,-34} {1,6}s {2,-10} {3,-30} {4}" -f $k, $r.Seconds, $r.Kind, $name, $tag
}
