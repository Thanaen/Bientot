<#
.SYNOPSIS
Measures startup, idle CPU and idle working set of worktime.exe.

.DESCRIPTION
SC-007 budgets startup with the built-in activity source, and with ActivityWatch already available when it
is the selected source. WorkTime runs with the built-in source by default. With -ActivityWatch it runs
against a local ActivityWatch stub that answers info, buckets and AFK events with the shapes the Python
aw-server returns. The summary lists each log event, and each stub request,
between process start and startup.tray_ready.

.PARAMETER ExecutablePath
The release worktime.exe, for example target\release\worktime.exe.

.PARAMETER ActivityWatch
Selects ActivityWatch instead of the built-in source, served by the stub unless ActivityWatchUrl is given.

.PARAMETER ActivityWatchUrl
Selects ActivityWatch at this server instead of the stub, for example http://localhost:5600/.

.PARAMETER Release
Release VM run for SC-006 and SC-007: a 15-minute idle window, failing at the spec budgets. Run it once
with the built-in source and once with -ActivityWatch.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $ExecutablePath,

    [ValidateRange(10, 3600)]
    [int] $IdleSeconds = 30,

    [ValidateRange(0, 300)]
    [int] $SettleSeconds = 5,

    [ValidateRange(0.1, 100)]
    [double] $CpuLimitPercent = 2,

    [ValidateRange(10, 4096)]
    [double] $WorkingSetLimitMiB = 200,

    [ValidateRange(0.5, 60)]
    [double] $StartupLimitSeconds = 6,

    [switch] $ActivityWatch,

    [ValidatePattern('^https?://[^/?#@]+/?$')]
    [string] $ActivityWatchUrl,

    [switch] $Release
)

$ErrorActionPreference = 'Stop'

$CpuBudgetPercent = 0.5
$WorkingSetBudgetMiB = 150
$StartupBudgetSeconds = 3

function Measure-IdleSamples([object[]] $Samples, [int] $ProcessorCount) {
    $first = $Samples[0]
    $last = $Samples[-1]
    $wallSeconds = $last.ElapsedSeconds - $first.ElapsedSeconds
    [pscustomobject]@{
        WallSeconds = $wallSeconds
        CpuPercent = ($last.CpuSeconds - $first.CpuSeconds) / $wallSeconds / $ProcessorCount * 100
        PeakWorkingSetMiB = ($Samples.WorkingSetBytes | Measure-Object -Maximum).Maximum / 1MB
        LastWorkingSetMiB = $last.WorkingSetBytes / 1MB
        PeakPrivateMiB = ($Samples.PrivateBytes | Measure-Object -Maximum).Maximum / 1MB
    }
}

function Get-BudgetStatus([double] $Value, [double] $Budget, [double] $Limit) {
    if ($Value -gt $Limit) { 'fail' }
    elseif ($Value -gt $Budget) { 'over budget' }
    else { 'pass' }
}

function Start-ActivityWatchStub {
    $probe = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $probe.Start()
    $port = $probe.LocalEndpoint.Port
    $probe.Stop()
    # http.sys lets a non-administrator listen on localhost, and then answers only requests sent to that host name.
    $url = "http://localhost:$port/"
    $listener = [Net.HttpListener]::new()
    $listener.Prefixes.Add($url)
    $listener.Start()
    $requests = [Collections.Concurrent.ConcurrentQueue[object]]::new()
    $shell = [PowerShell]::Create()
    $null = $shell.AddScript({
        param($Listener, $Requests)
        $info = '{"hostname":"worktime-test-host","version":"v0.12.3","testing":false}'
        $buckets = '{"aw-watcher-afk_worktime-test-host":{"id":"aw-watcher-afk_worktime-test-host",' +
            '"type":"afkstatus","client":"aw-watcher-afk","hostname":"worktime-test-host",' +
            '"created":"2026-07-27T07:00:00+00:00"}}'
        while ($Listener.IsListening) {
            try { $context = $Listener.GetContext() } catch { break }
            $path = $context.Request.Url.AbsolutePath
            $status = 200
            if ($path -eq '/api/0/info') { $body = $info }
            elseif ($path -eq '/api/0/buckets/') { $body = $buckets }
            elseif ($path -match '^/api/0/buckets/[^/]+/events$') { $body = '[]' }
            else { $status = 404; $body = '{}' }
            $Requests.Enqueue([pscustomobject]@{ AtUtc = [DateTime]::UtcNow; Path = $path; Status = $status })
            try {
                $bytes = [Text.Encoding]::UTF8.GetBytes($body)
                $context.Response.StatusCode = $status
                $context.Response.ContentType = 'application/json'
                $context.Response.ContentLength64 = $bytes.Length
                $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
                $context.Response.Close()
            }
            catch { }
        }
    }).AddArgument($listener).AddArgument($requests)
    [pscustomobject]@{ Url = $url; Listener = $listener; Requests = $requests; Shell = $shell; Run = $shell.BeginInvoke() }
}

# Closing the listener ends the stub's GetContext loop.
function Stop-ActivityWatchStub([object] $Stub) {
    $Stub.Listener.Close()
    $null = $Stub.Run.AsyncWaitHandle.WaitOne(5000)
    $Stub.Shell.Dispose()
}

# A line without its closing brace is one WorkTime is still writing.
function Read-LogEvents {
    Get-ChildItem -LiteralPath $logDirectory -Filter 'worktime-*.jsonl' -File -ErrorAction SilentlyContinue |
        Get-Content |
        Where-Object { $_.EndsWith('}') } |
        ForEach-Object {
            $root = [Text.Json.JsonDocument]::Parse($_).RootElement
            [pscustomobject]@{
                AtUtc = $root.GetProperty('TimestampUtc').GetDateTimeOffset().UtcDateTime
                Name = $root.GetProperty('Event').GetString()
                Level = $root.GetProperty('Level').GetString()
            }
        }
}

if ($Release) {
    if (-not $PSBoundParameters.ContainsKey('IdleSeconds')) { $IdleSeconds = 900 }
    $CpuLimitPercent = $CpuBudgetPercent
    $WorkingSetLimitMiB = $WorkingSetBudgetMiB
    $StartupLimitSeconds = $StartupBudgetSeconds
}

$executablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
if ([IO.Path]::GetFileName($executablePath) -ne 'worktime.exe') {
    throw 'Expected worktime.exe.'
}

$harness = [Diagnostics.Stopwatch]::StartNew()
$dataDirectory = Join-Path ([IO.Path]::GetTempPath()) "WorkTimePerformance-$([guid]::NewGuid().ToString('N'))"
$logDirectory = Join-Path $dataDirectory 'logs'
New-Item -ItemType Directory -Path $dataDirectory | Out-Null

function Assert-Running {
    if ($process.HasExited) {
        throw "worktime.exe exited with code $($process.ExitCode). Evidence: $dataDirectory"
    }
}

$process = $null
$stub = $null
try {
    # Saved settings run the whole startup instead of the first-run onboarding.
    # Startup records the source they name, since the new database has no selection yet.
    $settings = @{ schemaVersion = 1; activitySource = 'built-in' }
    if ($ActivityWatch -or $ActivityWatchUrl) {
        if ($ActivityWatchUrl) {
            $sourceUrl = $ActivityWatchUrl
        }
        else {
            $stub = Start-ActivityWatchStub
            $sourceUrl = $stub.Url
        }
        $settings = @{ schemaVersion = 1; activitySource = 'activitywatch'; activityWatch = @{ baseUrl = $sourceUrl } }
    }
    $settings | ConvertTo-Json -Compress | Set-Content -Path (Join-Path $dataDirectory 'settings.json') -Encoding ascii

    $process = Start-Process `
        -FilePath $executablePath `
        -WorkingDirectory (Split-Path -Parent $executablePath) `
        -ArgumentList @('--worktime-windows-startup', '--data-dir', "`"$dataDirectory`"") `
        -PassThru
    try {
        $startedAt = $process.StartTime.ToUniversalTime()
    }
    catch {
        Assert-Running
        throw
    }

    $readyAt = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ($null -eq $readyAt) {
        Assert-Running
        $ready = Read-LogEvents | Where-Object Name -EQ 'startup.tray_ready' | Select-Object -First 1
        if ($null -ne $ready) {
            $readyAt = $ready.AtUtc
        }
        elseif ([DateTime]::UtcNow -gt $deadline) {
            throw "WorkTime did not record startup.tray_ready within 30 seconds. Evidence: $dataDirectory"
        }
        else {
            Start-Sleep -Milliseconds 100
        }
    }
    $startupSeconds = ($readyAt - $startedAt).TotalSeconds
    $timeline = @(Read-LogEvents | Where-Object AtUtc -LE $readyAt | ForEach-Object {
        [pscustomobject]@{ AtUtc = $_.AtUtc; Source = 'log'; Text = "$($_.Name) ($($_.Level))" }
    })
    if ($stub) {
        $startupRequests = @($stub.Requests.ToArray() | Where-Object AtUtc -LE $readyAt)
        if (-not ($startupRequests | Where-Object { $_.Path -like '/api/0/buckets/*/events' -and $_.Status -eq 200 })) {
            throw ("The ActivityWatch stub served no AFK events before startup.tray_ready ($($startupRequests.Count) " +
                "requests), so the startup measure did not include a refresh against a healthy ActivityWatch. " +
                "Evidence: $dataDirectory")
        }
        $timeline += $startupRequests | ForEach-Object {
            [pscustomobject]@{ AtUtc = $_.AtUtc; Source = 'stub'; Text = "GET $($_.Path) $($_.Status)" }
        }
    }

    Start-Sleep -Seconds $SettleSeconds
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $samples = [Collections.Generic.List[object]]::new()
    for ($second = 0; $second -le $IdleSeconds; $second++) {
        while ($clock.Elapsed.TotalSeconds -lt $second) { Start-Sleep -Milliseconds 20 }
        Assert-Running
        $process.Refresh()
        $samples.Add([pscustomobject]@{
            ElapsedSeconds = $clock.Elapsed.TotalSeconds
            CpuSeconds = $process.TotalProcessorTime.TotalSeconds
            WorkingSetBytes = $process.WorkingSet64
            PrivateBytes = $process.PrivateMemorySize64
        })
    }
    $lifetimePeakMiB = $process.PeakWorkingSet64 / 1MB
}
finally {
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
    if ($stub) {
        Stop-ActivityWatchStub $stub
    }
}

$idle = Measure-IdleSamples $samples.ToArray() ([Environment]::ProcessorCount)
$results = @(
    [pscustomobject]@{ Name = 'Startup to startup.tray_ready'; Value = $startupSeconds; Budget = $StartupBudgetSeconds; Limit = $StartupLimitSeconds; Unit = 's'; Format = '{0:N2}' }
    [pscustomobject]@{ Name = 'Average idle CPU'; Value = $idle.CpuPercent; Budget = $CpuBudgetPercent; Limit = $CpuLimitPercent; Unit = '%'; Format = '{0:N3}' }
    [pscustomobject]@{ Name = 'Peak idle working set'; Value = $idle.PeakWorkingSetMiB; Budget = $WorkingSetBudgetMiB; Limit = $WorkingSetLimitMiB; Unit = 'MiB'; Format = '{0:N1}' }
)
foreach ($result in $results) {
    $result | Add-Member Status (Get-BudgetStatus $result.Value $result.Budget $result.Limit)
}

$summary = @(
    "### WorkTime performance ($(if ($Release) { 'release VM' } else { 'CI smoke' }), $(if ($settings.activitySource -eq 'built-in') { 'built-in source' } else { 'ActivityWatch' }))"
    ''
    '| Measure | Result | Spec budget | Fail above | Status |'
    '| --- | --- | --- | --- | --- |'
)
foreach ($result in $results) {
    $summary += '| {0} | {1} {4} | {2} {4} | {3} {4} | {5} |' -f $result.Name,
        ($result.Format -f $result.Value), $result.Budget, $result.Limit, $result.Unit, $result.Status
}
$summary += @(
    ''
    ('Idle window {0:N0} s after a {1} s settle, sampled every second, on {2} logical processors ({3}). ' +
        'Last working set {4:N1} MiB, lifetime peak {5:N1} MiB, peak private bytes {6:N1} MiB. Harness time {7:N0} s.') -f
        $idle.WallSeconds, $SettleSeconds, [Environment]::ProcessorCount, [Environment]::OSVersion.VersionString,
        $idle.LastWorkingSetMiB, $lifetimePeakMiB, $idle.PeakPrivateMiB, $harness.Elapsed.TotalSeconds
    ''
    $(if ($stub) {
        'The ActivityWatch stub at {0} served {1} requests before startup.tray_ready and {2} in total.' -f
            $stub.Url, $startupRequests.Count, $stub.Requests.Count
    }
    elseif ($ActivityWatchUrl) {
        "WorkTime used the ActivityWatch server at $ActivityWatchUrl, which the harness does not check."
    }
    else {
        'WorkTime used the built-in activity source.'
    })
    ''
    '| Since process start | Source | Event |'
    '| --- | --- | --- |'
    '| 0.00 s | process | start |'
)
foreach ($entry in $timeline | Sort-Object AtUtc) {
    $summary += '| {0:N2} s | {1} | {2} |' -f ($entry.AtUtc - $startedAt).TotalSeconds, $entry.Source, $entry.Text
}
$summary | Write-Host
if ($env:GITHUB_STEP_SUMMARY) {
    $summary | Add-Content -Path $env:GITHUB_STEP_SUMMARY
}
Remove-Item -LiteralPath $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue

foreach ($result in $results | Where-Object Status -EQ 'over budget') {
    Write-Host "::warning::$($result.Name) is over the release budget of $($result.Budget) $($result.Unit)."
}
$failures = @($results | Where-Object Status -EQ 'fail')
if ($failures) {
    throw "Performance limits exceeded: $(($failures.Name) -join ', ')."
}
