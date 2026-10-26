<#
.SYNOPSIS
  Misst RAM und CPU von Panco unter Windows (inkl. WebView2-Prozesse des Overlays).

.DESCRIPTION
  Panco muss laufen. Das Skript misst nacheinander:
    1. Leerlauf (kein Fenster; im Wake-Word-Modus mit offenem Mikro)
    2. Overlay offen (öffnet es per Strg+Leertaste)
    3. Overlay wieder geschlossen
    Mit -Voice zusätzlich:
    4. Sprachbefehl (du sagst "Panco, öffne …" – Whisper wird geladen)
    5. Nach der Freigabe von Whisper (wartet "keep_model_secs" + 5 s)
  RAM = "Arbeitsspeicher (privater Arbeitssatz)" wie im Task-Manager, summiert über Panco und
  alle Kindprozesse (msedgewebview2.exe). CPU = Prozent EINES Kerns im Mittel über die Phase.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Seconds 30
#>
param(
  [int]$Seconds = 20,
  [switch]$Manual,  # Hotkeys nicht simulieren, sondern selbst drücken
  [switch]$Voice,   # Sprach-Phasen mitmessen
  [int]$KeepModelSecs = 30
)

$ErrorActionPreference = 'Stop'

function Get-PancoTree {
  $root = Get-Process -Name panco -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $root) { throw "Panco läuft nicht. Bitte zuerst starten." }
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
  $ids = New-Object System.Collections.Generic.List[int]
  $ids.Add($root.Id)
  $i = 0
  while ($i -lt $ids.Count) {
    $parent = $ids[$i]
    foreach ($p in $all) { if ($p.ParentProcessId -eq $parent -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) } }
    $i++
  }
  return $ids
}

function Get-Sample {
  $ids = Get-PancoTree
  $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object { $ids -contains [int]$_.IDProcess }
  $privateMB = ($perf | Measure-Object -Property WorkingSetPrivate -Sum).Sum / 1MB
  $cpu = 0.0
  $names = @()
  foreach ($id in $ids) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if ($p) { $cpu += $p.TotalProcessorTime.TotalSeconds; $names += $p.ProcessName }
  }
  [pscustomobject]@{ PrivateMB = $privateMB; CpuSeconds = $cpu; Count = $ids.Count; Names = ($names | Sort-Object -Unique) -join ', ' }
}

function Measure-Phase([string]$Label) {
  $a = Get-Sample
  $sw = [Diagnostics.Stopwatch]::StartNew()
  Start-Sleep -Seconds $Seconds
  $b = Get-Sample
  $pct = ($b.CpuSeconds - $a.CpuSeconds) / $sw.Elapsed.TotalSeconds * 100
  $row = [pscustomobject]@{
    Phase        = $Label
    'RAM privat (MB)' = [math]::Round($b.PrivateMB, 1)
    'CPU (% 1 Kern)'  = [math]::Round($pct, 2)
    Prozesse     = "$($b.Count): $($b.Names)"
  }
  $row | Format-Table -AutoSize | Out-String | Write-Host
  return $row
}

function Press-Hotkey([string]$Hint) {
  if ($Manual) {
    Read-Host "$Hint, dann Enter drücken"
  } else {
    # Strg+Leertaste simulieren (löst den globalen Hotkey aus)
    $ws = New-Object -ComObject WScript.Shell
    $ws.SendKeys('^ ')
    Start-Sleep -Seconds 3
  }
}

Write-Host "Panco-Messung: $Seconds s pro Phase`n"
$results = @()
$results += Measure-Phase 'Leerlauf (kein Fenster)'
Press-Hotkey 'Overlay öffnen (Strg+Leertaste)'
$results += Measure-Phase 'Overlay offen'
Press-Hotkey 'Overlay schließen (Strg+Leertaste oder Esc)'
Start-Sleep -Seconds 5   # WebView2 braucht einen Moment zum Beenden
$results += Measure-Phase 'Overlay geschlossen'

if ($Voice) {
  Read-Host "Gleich: sag nach Enter deutlich `"Panco, öffne <ein Spiel>`" (oder halte Strg+Alt+Leertaste und sprich). Enter"
  $results += Measure-Phase 'Sprachbefehl (Whisper geladen)'
  Write-Host "Warte $($KeepModelSecs + 5) s, bis Whisper freigegeben ist ..."
  Start-Sleep -Seconds ($KeepModelSecs + 5)
  $results += Measure-Phase 'Nach Freigabe von Whisper'
}

Write-Host "Zusammenfassung (zum Kopieren):"
$results | ConvertTo-Json -Compress | Write-Host
