# Explicit local Windows process GPU-engine observation. No recording or upload.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateRange(1,2147483647)][int]$TargetProcessId,
    [ValidateRange(2,300)][int]$Samples = 12,
    [ValidateRange(1000,4000)][int]$IntervalMs = 2000,
    [Parameter(Mandatory=$true)][string]$OutputPath
)
$ErrorActionPreference = 'Stop'
$target = Get-Process -Id $TargetProcessId
$startTicks = $target.StartTime.ToUniversalTime().Ticks
function Confirm-Target {
    $p = Get-Process -Id $TargetProcessId
    if ($p.StartTime.ToUniversalTime().Ticks -ne $startTicks) {
        throw 'Target process was replaced'
    }
}
$clock = [System.Diagnostics.Stopwatch]::StartNew()
$observations = [System.Collections.Generic.List[object]]::new()
$pattern = '^pid_' + $TargetProcessId + '_'
for ($i = 0; $i -lt $Samples; $i++) {
    Confirm-Target
    $queryStart = $clock.ElapsedMilliseconds
    $status = 'provider_unavailable'
    $values = @()
    try {
        $rows = @(Get-CimInstance -Namespace root/cimv2 -ClassName Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -OperationTimeoutSec 5)
        $matching = @($rows | Where-Object { $_.Name -match $pattern })
        $status = 'no_matching_engine'
        if ($matching.Count -gt 0) {
            $status = 'measured'
            $values = @(
                foreach ($row in $matching) {
                    $value = $row.UtilizationPercentage
                    if ($null -eq $value -or $value -lt 0 -or $value -gt 100) {
                        throw 'Invalid engine percentage'
                    }
                    [int]$value
                }
            )
        }
    } catch {
        $status = 'provider_unavailable'
        $values = @()
    }
    Confirm-Target
    $end = $clock.ElapsedMilliseconds
    $observations.Add([ordered]@{
        elapsed_ms = $end
        captured_at_utc = [DateTime]::UtcNow.ToString('o')
        query_duration_ms = $end - $queryStart
        status = $status
        engine_percentages = @($values)
    })
    if ($i -lt $Samples - 1) {
        Start-Sleep -Milliseconds $IntervalMs
    }
}
# Do not export PID, process name, path, adapter identifiers or exception text.
$report = [ordered]@{
    schema_version = 1
    sample_kind = 'windows_process_gpu_engine_series'
    source = 'windows_cim_formatted_gpu_engine'
    target_identity_stable = $true
    nominal_interval_ms = $IntervalMs
    samples = @($observations.ToArray())
}
$json = $report | ConvertTo-Json -Depth 6
[System.IO.File]::WriteAllText($OutputPath, $json, [System.Text.UTF8Encoding]::new($false))
