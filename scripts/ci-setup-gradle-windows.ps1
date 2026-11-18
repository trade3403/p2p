# V0.13: provision an official, SHA-256-verified Gradle distribution
# to a persistent per-user cache on the self-hosted Windows Runner.
# This removes the GitHub Actions setup-gradle CDN dependency that timed out.
$ErrorActionPreference = 'Stop'
$version = '8.10.2'
$expectedSha256 = '31c55713e40233a8303827ceb42ca48a47267a0ad4bab9177123121e71524c26'
$root = Join-Path $env:USERPROFILE '.gradle\ci-distributions'
$home = Join-Path $root "gradle-$version"
$exe = Join-Path $home 'bin\gradle.bat'
$zip = Join-Path $root "gradle-$version-bin.zip"
$stage = Join-Path $root "unpack-$version"
New-Item -ItemType Directory -Force -Path $root | Out-Null

if (-not (Test-Path -LiteralPath $exe)) {
    $haveValidZip = $false
    if (Test-Path -LiteralPath $zip) {
        $haveValidZip = ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant() -eq $expectedSha256)
        if (-not $haveValidZip) { Remove-Item -LiteralPath $zip -Force }
    }
    if (-not $haveValidZip) {
        # A verified mirror is only a fallback when official endpoints are unreachable.
        $sources = @(
            "https://downloads.gradle.org/distributions/gradle-$version-bin.zip",
            "https://services.gradle.org/distributions/gradle-$version-bin.zip",
            "https://mirrors.huaweicloud.com/gradle/gradle-$version-bin.zip"
        )
        foreach ($source in $sources) {
            try {
                Write-Host "Fetching Gradle $version from $source"
                Invoke-WebRequest -Uri $source -OutFile $zip -UseBasicParsing -TimeoutSec 240
                $digest = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($digest -ne $expectedSha256) {
                    throw "Gradle distribution SHA-256 mismatch (rejecting archive)"
                }
                $haveValidZip = $true
                break
            } catch {
                Write-Warning ("Gradle endpoint failed: " + $_.Exception.Message)
                if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
            }
        }
    }
    if (-not $haveValidZip) { throw "Cannot download verified Gradle $version via any configured endpoint" }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    if (Test-Path -LiteralPath $home) { Remove-Item -LiteralPath $home -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Expand-Archive -LiteralPath $zip -DestinationPath $stage -Force
    $unpacked = Join-Path $stage "gradle-$version"
    if (-not (Test-Path -LiteralPath (Join-Path $unpacked 'bin\gradle.bat'))) {
        throw "Verified Gradle archive has unexpected layout"
    }
    Move-Item -LiteralPath $unpacked -Destination $home
    Remove-Item -LiteralPath $stage -Recurse -Force
}
if (-not (Test-Path -LiteralPath $exe)) { throw "Gradle $version executable missing" }
$bin = Split-Path -Parent $exe
[System.IO.File]::AppendAllText($env:GITHUB_PATH, $bin + [Environment]::NewLine,
    (New-Object System.Text.UTF8Encoding($false)))
Write-Host "Using cached Gradle $version at $exe"
& $exe --version
if ($LASTEXITCODE -ne 0) { throw "Gradle --version failed" }
