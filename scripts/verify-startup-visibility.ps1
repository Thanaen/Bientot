param(
    [Parameter(Mandatory)]
    [string]$ExecutablePath,
    [ValidateSet('Startup', 'Manual', 'Failure')]
    [string]$Mode = 'Startup',
    [ValidateRange(3, 60)]
    [int]$ObservationSeconds = 8,
    [string]$OutputDirectory = (Join-Path ([IO.Path]::GetTempPath()) "WorkTime Visibility $([Guid]::NewGuid().ToString('N'))")
)

$ErrorActionPreference = 'Stop'
$ExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
if ([IO.Path]::GetFileName($ExecutablePath) -ne 'worktime.exe') { throw 'Expected worktime.exe.' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$dataDirectory = Join-Path $OutputDirectory 'data'
if (Test-Path -LiteralPath $dataDirectory) { throw 'Use a fresh output directory for each run.' }
if ($Mode -eq 'Failure') { [IO.File]::WriteAllText($dataDirectory, 'This file prevents database directory creation.') }
$arguments = "--demo --data-dir `"$dataDirectory`""
if ($Mode -ne 'Manual') { $arguments += ' --worktime-windows-startup' }

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class WorkTimeVisibilityProbe
{
    public sealed class Observation
    {
        public uint ProcessId;
        public bool ProcessAlive;
        public int ShowEvents;
        public int VisibleWindows;
        public int FailureWindows;
        public string[] ShownTitles = Array.Empty<string>();
    }

    public static Observation Run(string executable, string arguments, int seconds)
    {
        var startup = new StartupInfo { Size = Marshal.SizeOf<StartupInfo>(), Flags = 1, ShowWindow = 0 };
        if (!CreateProcess(executable, new StringBuilder("\"" + executable + "\" " + arguments),
            IntPtr.Zero, IntPtr.Zero, false, 4, IntPtr.Zero, System.IO.Path.GetDirectoryName(executable),
            ref startup, out var process))
            throw new Win32Exception(Marshal.GetLastWin32Error());

        var result = new Observation { ProcessId = process.ProcessId };
        var titles = new List<string>();
        IntPtr hook = IntPtr.Zero;
        WinEvent callback = (h, eventId, window, objectId, childId, threadId, time) =>
        {
            if (window == IntPtr.Zero || objectId != 0 || childId != 0 || GetAncestor(window, 2) != window) return;
            result.ShowEvents++;
            titles.Add(Title(window));
        };

        try
        {
            // The process stays suspended until the observer is installed, so an early flash cannot precede the hook.
            hook = SetWinEventHook(0x8002, 0x8002, IntPtr.Zero, callback, process.ProcessId, 0, 0);
            if (hook == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (ResumeThread(process.Thread) == uint.MaxValue) throw new Win32Exception(Marshal.GetLastWin32Error());
            var timer = Stopwatch.StartNew();
            while (timer.Elapsed.TotalSeconds < seconds && WaitForSingleObject(process.Process, 0) == 258)
            {
                while (PeekMessage(out var message, IntPtr.Zero, 0, 0, 1))
                {
                    TranslateMessage(ref message);
                    DispatchMessage(ref message);
                }
                Thread.Sleep(10);
            }
            while (PeekMessage(out var message, IntPtr.Zero, 0, 0, 1))
            {
                TranslateMessage(ref message);
                DispatchMessage(ref message);
            }
            result.ProcessAlive = WaitForSingleObject(process.Process, 0) == 258;
            EnumWindows((window, state) =>
            {
                GetWindowThreadProcessId(window, out var owner);
                if (owner != process.ProcessId) return true;
                if (!IsWindowVisible(window)) return true;
                result.VisibleWindows++;
                // The startup failure window is the only one titled with the app's name: the flyout has no title.
                if (Title(window) == "Bient\u00f4t") result.FailureWindows++;
                return true;
            }, IntPtr.Zero);
            result.ShownTitles = titles.ToArray();
            return result;
        }
        finally
        {
            if (hook != IntPtr.Zero) UnhookWinEvent(hook);
            if (WaitForSingleObject(process.Process, 0) == 258)
            {
                if (!TerminateProcess(process.Process, 0)) throw new Win32Exception(Marshal.GetLastWin32Error());
                if (WaitForSingleObject(process.Process, 5000) != 0) throw new InvalidOperationException("Test process did not stop.");
            }
            CloseHandle(process.Thread);
            CloseHandle(process.Process);
            GC.KeepAlive(callback);
        }
    }

    private static string Title(IntPtr window)
    {
        var title = new StringBuilder(512);
        GetWindowText(window, title, title.Capacity);
        return title.ToString();
    }

    private delegate void WinEvent(IntPtr hook, uint eventId, IntPtr window, int objectId, int childId, uint threadId, uint time);
    private delegate bool WindowVisitor(IntPtr window, IntPtr state);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo
    {
        public int Size;
        public string Reserved, Desktop, Title;
        public uint X, Y, Width, Height, CharsX, CharsY, FillAttribute, Flags;
        public ushort ShowWindow, ReservedBytes;
        public IntPtr ReservedPointer, StandardInput, StandardOutput, StandardError;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInfo { public IntPtr Process, Thread; public uint ProcessId, ThreadId; }

    [StructLayout(LayoutKind.Sequential)]
    private struct Message
    {
        public IntPtr Window;
        public uint Id;
        public UIntPtr WParam;
        public IntPtr LParam;
        public uint Time;
        public int X, Y;
        public uint Private;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CreateProcess(string application, StringBuilder command, IntPtr processAttributes, IntPtr threadAttributes,
        bool inheritHandles, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInfo process);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll")] private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool TerminateProcess(IntPtr process, uint exitCode);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr SetWinEventHook(uint minimum, uint maximum, IntPtr module, WinEvent callback, uint processId, uint threadId, uint flags);
    [DllImport("user32.dll")] private static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] private static extern bool EnumWindows(WindowVisitor visitor, IntPtr state);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder text, int maximum);
    [DllImport("user32.dll")] private static extern bool PeekMessage(out Message message, IntPtr window, uint minimum, uint maximum, uint flags);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref Message message);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr DispatchMessage(ref Message message);
}
'@

$observation = [WorkTimeVisibilityProbe]::Run($ExecutablePath, $arguments, $ObservationSeconds)

$observation | ConvertTo-Json | Tee-Object -FilePath (Join-Path $OutputDirectory 'visibility.json')
if (-not $observation.ProcessAlive) { throw 'WorkTime exited during observation.' }
if ($Mode -eq 'Failure') {
    if ($observation.FailureWindows -ne 1) { throw 'Initialization failure did not show the startup failure window.' }
}
else {
    if ($observation.FailureWindows -ne 0) { throw 'WorkTime failed during startup.' }
    if (-not (Test-Path -LiteralPath (Join-Path $dataDirectory 'worktime.db'))) { throw 'Database initialization did not run.' }
    if ($Mode -eq 'Startup' -and ($observation.ShowEvents -ne 0 -or $observation.VisibleWindows -ne 0)) {
        throw 'Automatic startup showed a top-level window.'
    }
    if ($Mode -eq 'Manual' -and ($observation.ShowEvents -lt 1 -or $observation.VisibleWindows -lt 1)) {
        throw 'Manual startup did not show its window.'
    }
}
Write-Output "PASS $Mode visibility. Test process stopped. Evidence in $OutputDirectory"
