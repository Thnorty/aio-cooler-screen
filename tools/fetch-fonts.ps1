# Download official, OFL-licensed TTFs when building; no font binaries committed.
# Requires access to GitHub's public API and raw GitHub content.
$ErrorActionPreference = 'Stop'
$dest = Join-Path $PSScriptRoot '..\assets\fonts'
New-Item -ItemType Directory -Force $dest, "$dest\licenses" | Out-Null
$families = [ordered]@{
  'GoogleSans'='googlesans'; 'GoogleSansFlex'='googlesansflex';
  'Inter'='inter'; 'Rajdhani'='rajdhani'; 'Orbitron'='orbitron';
  'JetBrainsMono'='jetbrainsmono'; 'RobotoCondensed'='robotocondensed'; 'Oswald'='oswald'
}
$headers = @{ 'User-Agent'='AIO-Cooler-Screen-Font-Build'; 'Accept'='application/vnd.github+json' }
if ($env:GH_TOKEN) { $headers['Authorization']="Bearer $env:GH_TOKEN" }
function Download-File([string]$url,[string]$file) {
  Invoke-WebRequest -Uri $url -Headers $headers -OutFile $file
  if ((Get-Item $file).Length -lt 1000) { throw "Suspiciously small file: $file" }
}
foreach ($family in $families.Keys) {
  $slug=$families[$family]
  $api="https://api.github.com/repos/google/fonts/contents/ofl/$slug"
  Write-Host "Retrieving $family from official google/fonts repository"
  $entries=Invoke-RestMethod -Uri $api -Headers $headers
  $license=$entries | Where-Object { $_.name -eq 'OFL.txt' } | Select-Object -First 1
  if (-not $license) { throw "Missing SIL OFL for $family" }
  Download-File $license.download_url "$dest\licenses\$family-OFL.txt"
  $files=@($entries | Where-Object { $_.name -match '\.ttf$' })
  $static=$entries | Where-Object { $_.name -eq 'static' -and $_.type -eq 'dir' } | Select-Object -First 1
  if ($static) { $files += @(Invoke-RestMethod -Uri $static.url -Headers $headers | Where-Object { $_.name -match '\.ttf$' }) }
  $regular=$files | Where-Object { $_.name -match '(?i)(-Regular|\[.*\])\.ttf$' -and $_.name -notmatch 'Italic' } | Sort-Object @{Expression={ if ($_.name -match '-Regular\.ttf$') {0} else {1} }} | Select-Object -First 1
  $bold=$files | Where-Object { $_.name -match '(?i)(-Bold|\[.*\])\.ttf$' -and $_.name -notmatch 'Italic' } | Sort-Object @{Expression={ if ($_.name -match '-Bold\.ttf$') {0} else {1} }} | Select-Object -First 1
  if (-not $regular -or -not $bold) { throw "No usable TTF files found for $family in $api" }
  Download-File $regular.download_url "$dest\$family-Regular.ttf"
  Download-File $bold.download_url "$dest\$family-Bold.ttf"
}
Write-Host 'All 8 font families downloaded with their OFL files.'
