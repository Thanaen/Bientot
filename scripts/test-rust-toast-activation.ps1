#Requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ExecutablePath,
    [ValidateRange(1, 120)][int]$TimeoutSeconds = 30
)

# Proves the cold start of a toast click without an interactive session: with WorkTime stopped,
# COM must start the portable build for the click and hand it to the class it registers.
# A manual smoke test for a Windows session or runner; no CI job runs it yet.
$ErrorActionPreference = 'Stop'
# reg.exe exits non-zero for a key that does not exist; the restore below checks exit codes itself.
$PSNativeCommandUseErrorActionPreference = $false
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$processName = [IO.Path]::GetFileNameWithoutExtension($executable)
# PORTABLE_AUMID and PORTABLE_ACTIVATOR in crates/bientot-windows/src/notifications/portable.rs.
$aumid = 'Thanaen.WorkTime.Portable'
$activator = [guid]'8ddae643-7db0-465f-9139-725c4f57f1ed'
$registryKeys = @(
    "HKCU\Software\Classes\AppUserModelId\$aumid",
    "HKCU\Software\Classes\CLSID\{$activator}"
)
$testDirectory = Join-Path ([IO.Path]::GetTempPath()) "WorkTimeToastActivation-$([guid]::NewGuid().ToString('N'))"
$dataDirectory = Join-Path $testDirectory 'data'
$deliveryId = [guid]::NewGuid().ToString('N')

if (Get-Process -Name $processName -ErrorAction SilentlyContinue) {
    throw "Stop every running $processName first: a running WorkTime holds the activator, so COM would hand it the click and start nothing."
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace WorkTimeSmoke
{
    [ComImport, Guid("53E31837-6600-4A81-9395-75CFFE746F94"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface INotificationActivationCallback
    {
        void Activate(
            [MarshalAs(UnmanagedType.LPWStr)] string appUserModelId,
            [MarshalAs(UnmanagedType.LPWStr)] string invokedArgs,
            IntPtr data,
            uint count);
    }

    public static class ToastActivator
    {
        private const uint CLSCTX_LOCAL_SERVER = 0x4;

        [DllImport("ole32.dll")]
        private static extern int CoCreateInstance(
            [In] ref Guid clsid,
            IntPtr outer,
            uint context,
            [In] ref Guid iid,
            [MarshalAs(UnmanagedType.IUnknown)] out object instance);

        public static void Activate(Guid clsid, string appUserModelId, string arguments)
        {
            Guid iid = typeof(INotificationActivationCallback).GUID;
            object instance;
            Marshal.ThrowExceptionForHR(CoCreateInstance(ref clsid, IntPtr.Zero, CLSCTX_LOCAL_SERVER, ref iid, out instance));
            try
            {
                ((INotificationActivationCallback)instance).Activate(appUserModelId, arguments, IntPtr.Zero, 0);
            }
            finally
            {
                Marshal.ReleaseComObject(instance);
            }
        }
    }
}
'@

function Get-WorkTimeEvents {
    @(Get-ChildItem -LiteralPath (Join-Path $dataDirectory 'logs') -Filter 'worktime-*.jsonl' -File -ErrorAction SilentlyContinue |
        Get-Content | ForEach-Object { $_ | ConvertFrom-Json })
}

function Wait-WorkTimeEvent([string] $Description, [scriptblock] $Match, $Process) {
    $deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        $events = Get-WorkTimeEvents
        $failure = $events | Where-Object { $_.event -eq 'notification.registration_failed' } | Select-Object -Last 1
        if ($null -ne $failure) {
            Write-Host 'Notification failure event:'
            $failure | ConvertTo-Json -Depth 4 | Write-Host
            throw "WorkTime could not register its toasts. Evidence: $testDirectory"
        }
        if ($events | Where-Object $Match) {
            return
        }
        if ($null -ne $Process -and $Process.HasExited) {
            throw "WorkTime exited before $Description was recorded. Exit code $($Process.ExitCode). Evidence: $testDirectory"
        }
        Start-Sleep -Milliseconds 200
    } while ([DateTimeOffset]::UtcNow -lt $deadline)
    throw "No $Description was recorded within $TimeoutSeconds seconds. Evidence: $testDirectory"
}

function Stop-WorkTime {
    Get-Process -Name $processName -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $executable } |
        ForEach-Object {
            Stop-Process -Id $_.Id -Force
            $_.WaitForExit()
        }
}

New-Item -ItemType Directory -Path $dataDirectory | Out-Null
# Saved settings skip the onboarding, after which a first run would show the click.
Set-Content -Path (Join-Path $dataDirectory 'settings.json') -Value '{"schemaVersion":1,"activitySource":"built-in"}' -Encoding ascii

# The run below points the registration at this executable and folder; the user's own comes back.
$backups = foreach ($key in $registryKeys) {
    $file = Join-Path $testDirectory "$([guid]::NewGuid().ToString('N')).reg"
    & reg.exe export $key $file /y | Out-Null
    [pscustomobject]@{ Key = $key; File = if ($LASTEXITCODE -eq 0) { $file } else { $null } }
}
$testFailed = $true
try {
    $process = Start-Process -FilePath $executable -WorkingDirectory (Split-Path -Parent $executable) `
        -ArgumentList @('--data-dir', "`"$dataDirectory`"") -WindowStyle Hidden -PassThru
    Wait-WorkTimeEvent 'startup.tray_ready' { $_.event -eq 'startup.tray_ready' } $process
    Stop-WorkTime

    $command = (Get-ItemProperty -LiteralPath "HKCU:\Software\Classes\CLSID\{$activator}\LocalServer32").'(default)'
    Write-Host "Registered COM server: $command"
    if ($command -notlike "*`"$dataDirectory`"*") {
        throw "The registered COM server does not start WorkTime on $dataDirectory."
    }

    [WorkTimeSmoke.ToastActivator]::Activate($activator, $aumid, "deliveryId=$deliveryId")
    Wait-WorkTimeEvent 'notification.activated with origin cold' {
        $_.event -eq 'notification.activated' -and $_.properties.origin -eq 'cold'
    } $null
    Write-Host "notification.activated recorded with origin cold. Evidence: $testDirectory"
    $testFailed = $false
}
finally {
    Stop-WorkTime
    # Each key is restored even when another fails, and the failures are reported once all ran.
    $restoreFailures = foreach ($backup in $backups) {
        # Fails with 1 when the run never created the key too, so its exit code proves nothing.
        & reg.exe delete $backup.Key /f | Out-Null
        if ($null -ne $backup.File) {
            & reg.exe import $backup.File | Out-Null
            if ($LASTEXITCODE -ne 0) {
                "reg import $($backup.File) exited with $LASTEXITCODE"
            }
        }
    }
    if ($restoreFailures) {
        # A throw here would replace the test's own failure, so that one is only reported.
        $message = "The registry was not fully restored: $($restoreFailures -join '; '). Backups: $testDirectory"
        if ($testFailed) { Write-Warning $message } else { throw $message }
    }
}
