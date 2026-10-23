param(
    [Parameter(Mandatory)]
    [string]$PackagePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$resolvedPackagePath = (Resolve-Path -LiteralPath $PackagePath).Path
$extractionDirectory = Join-Path ([IO.Path]::GetTempPath()) "WorkTimePackage-$([guid]::NewGuid().ToString('N'))"

try {
    [IO.Compression.ZipFile]::ExtractToDirectory($resolvedPackagePath, $extractionDirectory)
    $manifestPath = Join-Path $extractionDirectory 'AppxManifest.xml'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "The package does not contain AppxManifest.xml."
    }

    [xml]$manifest = Get-Content -LiteralPath $manifestPath -Raw
    $executableDeclarations = @($manifest.SelectNodes(
        "//*[local-name()='Application' or local-name()='Extension' or local-name()='ExeServer'][@Executable]"))
    if ($executableDeclarations.Count -eq 0) {
        throw "The package manifest does not declare any executable paths."
    }

    $packageRoot = [IO.Path]::GetFullPath($extractionDirectory + [IO.Path]::DirectorySeparatorChar)
    foreach ($declaration in $executableDeclarations) {
        $executable = $declaration.GetAttribute('Executable')
        if ([string]::IsNullOrWhiteSpace($executable) -or $executable.Contains('$') -or [IO.Path]::IsPathRooted($executable)) {
            throw "The $($declaration.LocalName) executable path is unresolved or invalid: '$executable'."
        }

        $relativePath = $executable.Replace('\', [IO.Path]::DirectorySeparatorChar).Replace('/', [IO.Path]::DirectorySeparatorChar)
        $executablePath = [IO.Path]::GetFullPath((Join-Path $extractionDirectory $relativePath))
        if (-not $executablePath.StartsWith($packageRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "The $($declaration.LocalName) executable path escapes the package: '$executable'."
        }
        if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
            throw "The $($declaration.LocalName) executable does not exist in the package: '$executable'."
        }
    }

    Write-Host "Verified $($executableDeclarations.Count) executable declarations in $resolvedPackagePath."
}
finally {
    Remove-Item -LiteralPath $extractionDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
