#Requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][ValidateSet('zip', 'store')][string]$Channel,
    [Parameter(Mandatory)][string]$OutDir,
    [string]$Identity,
    [string]$Publisher,
    [string]$PublisherDisplayName,
    [string]$Version,
    [ValidateRange('NonNegative')][double]$MaxSizeMB = 0
)

# Packages a release build of worktime.exe. The channel only picks the download: the exe already
# carries its own, fixed by WORKTIME_CHANNEL when it was compiled. The identity overrides, for the
# Partner Center values, apply to the MSIX only.
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$packagingDirectory = Join-Path $repositoryRoot 'packaging/msix'
$executable = (Resolve-Path -LiteralPath $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$outputDirectory = (Resolve-Path -LiteralPath $OutDir).Path
$zipName = 'WorkTime-win-x64-portable.zip'
$msixName = 'WorkTime-win-x64-unsigned.msix'

[xml]$manifest = Get-Content -LiteralPath (Join-Path $packagingDirectory 'AppxManifest.xml') -Raw
$namespaces = [Xml.XmlNamespaceManager]::new($manifest.NameTable)
$namespaces.AddNamespace('m', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10')
$exeName = $manifest.SelectSingleNode('/m:Package/m:Applications/m:Application', $namespaces).GetAttribute('Executable')

function Find-SdkTool([string] $Name) {
    $onPath = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $onPath) {
        return $onPath.Source
    }
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $tool = Get-ChildItem -LiteralPath $sdkBin -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -as [version] } |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName "x64\$Name" } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
        Select-Object -First 1
    if ($null -eq $tool) {
        throw "$Name was not found on PATH or in a Windows 10 SDK under $sdkBin."
    }
    $tool
}

function Invoke-Native([string] $Tool, [string[]] $Arguments) {
    & $Tool @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "$([IO.Path]::GetFileName($Tool)) failed with exit code $LASTEXITCODE."
    }
}

function Copy-LicenseNotices([string] $Layout) {
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'LICENSE') -Destination (Join-Path $Layout 'LICENSE.txt')
    # The exe embeds the Lucide icons (ISC) and the IBM Plex fonts (SIL OFL), whose licenses ask
    # for their notice in every copy.
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'crates/worktime-app/assets/icons/LICENSE') `
        -Destination (Join-Path $Layout 'LICENSE-icons.txt')
    Copy-Item -LiteralPath (Join-Path $repositoryRoot 'crates/worktime-app/assets/fonts/OFL.txt') `
        -Destination (Join-Path $Layout 'LICENSE-fonts.txt')
    # The crates compiled into a release exe (worktime-app's `release` feature) and their licenses.
    # The generator fails on a license it does not allow, which stops the packaging. rustup picks
    # the toolchain from the current directory, so the run starts at the root to get
    # rust-toolchain.toml.
    Push-Location $repositoryRoot
    try {
        Invoke-Native 'cargo' @('run', '--locked', '--quiet', '--manifest-path', (Join-Path $repositoryRoot 'Cargo.toml'),
            '--package', 'worktime-notices', '--', (Join-Path $Layout 'THIRD-PARTY-NOTICES.txt'))
    }
    finally {
        Pop-Location
    }
}

function New-MsixPackage([string] $Staging, [string] $Destination) {
    $layout = Join-Path $Staging 'layout'
    New-Item -ItemType Directory -Path $layout | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $layout $exeName)
    Copy-Item -LiteralPath (Join-Path $packagingDirectory 'Images') -Destination $layout -Recurse
    Copy-LicenseNotices $layout

    $identityNode = $manifest.SelectSingleNode('/m:Package/m:Identity', $namespaces)
    if ($Identity) { $identityNode.SetAttribute('Name', $Identity) }
    if ($Publisher) { $identityNode.SetAttribute('Publisher', $Publisher) }
    if ($PublisherDisplayName) {
        $manifest.SelectSingleNode('/m:Package/m:Properties/m:PublisherDisplayName', $namespaces).InnerText = $PublisherDisplayName
    }
    $packageVersion = if ($Version) { $Version } else {
        $workspaceVersion = Select-String -LiteralPath (Join-Path $repositoryRoot 'Cargo.toml') -Pattern '^version = "(.+)"$' |
            Select-Object -First 1
        if ($null -eq $workspaceVersion) {
            throw 'No version was found in Cargo.toml; pass -Version.'
        }
        "$($workspaceVersion.Matches[0].Groups[1].Value).0"
    }
    if ($packageVersion -notmatch '^\d{1,5}(\.\d{1,5}){3}$' -or ($packageVersion.Split('.') | Where-Object { [int]$_ -gt 65535 })) {
        throw "The package version must be four numbers up to 65535: '$packageVersion'."
    }
    $identityNode.SetAttribute('Version', $packageVersion)
    $manifest.Save((Join-Path $layout 'AppxManifest.xml'))

    # The manifest names Images\Square44x44Logo.png, which exists only as scale-200 and
    # targetsize-24 variants: resources.pri maps the name to them. With the default scale
    # left at 100, makepri would split the scale-200 images into resources.scale-200.pri,
    # which a flat MSIX never loads.
    $makepri = Find-SdkTool 'makepri.exe'
    $priConfig = Join-Path $Staging 'priconfig.xml'
    Invoke-Native $makepri @('createconfig', '/cf', $priConfig, '/dq', 'lang-en-US_scale-200', '/pv', '10.0.0', '/o')
    Invoke-Native $makepri @('new', '/pr', $layout, '/cf', $priConfig, '/mn', (Join-Path $layout 'AppxManifest.xml'), '/of', (Join-Path $layout 'resources.pri'), '/o')
    $splitIndexes = Get-ChildItem -LiteralPath $layout -Filter 'resources*.pri' | Where-Object Name -ne 'resources.pri'
    if ($splitIndexes) {
        throw "makepri split resources into $($splitIndexes.Name -join ', '); the MSIX would not find them."
    }

    $makeappx = Find-SdkTool 'makeappx.exe'
    Invoke-Native $makeappx @('pack', '/d', $layout, '/p', $Destination, '/o')
    & (Join-Path $PSScriptRoot 'verify-package-executables.ps1') -PackagePath $Destination | Out-Host
}

function New-PortableZip([string] $Staging, [string] $Destination) {
    $layout = Join-Path $Staging 'portable'
    New-Item -ItemType Directory -Path $layout | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $layout $exeName)
    # PORTABLE_ICON in crates/worktime-windows/src/notifications/native.rs: the toasts' icon.
    Copy-Item -LiteralPath (Join-Path $packagingDirectory 'worktime.png') -Destination $layout
    Copy-LicenseNotices $layout
    Compress-Archive -Path (Join-Path $layout '*') -DestinationPath $Destination -CompressionLevel Optimal -Force
}

$packageName = $Channel -eq 'zip' ? $zipName : $msixName
$output = Join-Path $outputDirectory $packageName
$staging = Join-Path ([IO.Path]::GetTempPath()) "WorkTimePackage-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $staging | Out-Null
try {
    # Built aside and moved only once checked, so a rejected package never sits under its release name.
    $built = Join-Path $staging $packageName
    if ($Channel -eq 'zip') {
        New-PortableZip $staging $built
    }
    else {
        New-MsixPackage $staging $built
    }

    $sizeMB = (Get-Item -LiteralPath $built).Length / 1MB
    Write-Host ('{0}: {1:N2} MB' -f $packageName, $sizeMB)
    if ($MaxSizeMB -gt 0 -and $sizeMB -gt $MaxSizeMB) {
        throw ('{0} is {1:N2} MB, over the {2} MB budget.' -f $packageName, $sizeMB, $MaxSizeMB)
    }
    Move-Item -LiteralPath $built -Destination $output -Force
}
finally {
    Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction SilentlyContinue
}

# Both channels can be packaged into one folder, so the sums cover every download in it.
$checksums = Get-ChildItem -LiteralPath $outputDirectory -File |
    Where-Object Name -in $zipName, $msixName |
    Sort-Object Name |
    ForEach-Object { "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant())  $($_.Name)" }
# LF endings, so `sha256sum -c` accepts the file on any system.
Set-Content -LiteralPath (Join-Path $outputDirectory 'SHA256SUMS.txt') -Value (($checksums -join "`n") + "`n") -NoNewline -Encoding ascii
