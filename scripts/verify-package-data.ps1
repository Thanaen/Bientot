#Requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)]$Package,
    [ValidateRange(1, 120)][int]$TimeoutSeconds = 30
)

# Shows what the installed Store package sees of %LOCALAPPDATA%\Bientot, the folder of the
# portable and Setup.exe builds (#3). A command run in the package's context reads a file that
# the portable build left there, rewrites it and creates another; the script then reports where
# each write landed, and where a folder the package creates itself lands. The app itself is not started: it would run on the runner's profile.
$ErrorActionPreference = 'Stop'
$folder = Join-Path $env:LOCALAPPDATA 'Bientot'
$private = Join-Path $env:LOCALAPPDATA "Packages\$($Package.PackageFamilyName)\LocalCache\Local\Bientot"
$probe = Join-Path $env:RUNNER_TEMP "package-data-$([guid]::NewGuid().ToString('N')).txt"
New-Item -ItemType Directory -Force -Path $folder | Out-Null
Set-Content -LiteralPath (Join-Path $folder 'seed.txt') -Value 'portable' -NoNewline

# A folder the package creates itself, as on a PC that never ran the portable build.
$fresh = Join-Path $env:LOCALAPPDATA 'BientotProbe'
$freshPrivate = Join-Path $env:LOCALAPPDATA "Packages\$($Package.PackageFamilyName)\LocalCache\Local\BientotProbe"
if (Test-Path $fresh) { Remove-Item -Recurse -Force $fresh }

$commands = @(
    "mkdir `"$fresh`"",
    "echo fresh> `"$fresh\new.txt`"",
    "type `"$folder\seed.txt`" > `"$probe.read`"",
    "echo packaged> `"$folder\seed.txt`"",
    "echo new> `"$folder\new.txt`"",
    "type `"$folder\seed.txt`" > `"$probe.reread`"",
    "echo done> `"$probe`""
) -join ' & '
Invoke-CommandInDesktopPackage -PackageFamilyName $Package.PackageFamilyName -AppId 'App' -Command 'cmd.exe' -Args "/c $commands"

$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while (-not (Test-Path -LiteralPath $probe)) {
    if ((Get-Date) -gt $deadline) { throw 'The command in the package context did not finish.' }
    Start-Sleep -Milliseconds 250
}

$read = ([string](Get-Content -Raw -LiteralPath "$probe.read")).Trim()
$reread = ([string](Get-Content -Raw -LiteralPath "$probe.reread")).Trim()
$seed = ([string](Get-Content -Raw -LiteralPath (Join-Path $folder 'seed.txt'))).Trim()
Write-Host "Package read the portable file as: $read"
Write-Host "Package read its own rewrite as: $reread"
Write-Host "The portable folder's file now holds: $seed"
Write-Host "New file in the portable folder: $(Test-Path (Join-Path $folder 'new.txt'))"
Write-Host "Private folder $private exists: $(Test-Path $private)"
if (Test-Path $private) { Get-ChildItem -Recurse $private | ForEach-Object { Write-Host "  $($_.FullName)" } }
Write-Host "Folder the package created, in %LOCALAPPDATA%: $(Test-Path (Join-Path $fresh 'new.txt'))"
Write-Host "Folder the package created, in its LocalCache: $(Test-Path (Join-Path $freshPrivate 'new.txt'))"
if ($read -ne 'portable') { throw 'The package does not see the data of the portable build.' }
