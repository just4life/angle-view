# Downloads the background-removal model into models\birefnet-lite.safetensors.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\fetch-model.ps1
$ErrorActionPreference = "Stop"
$url = "https://github.com/just4life/angle-view/releases/latest/download/birefnet-lite.safetensors"
$dir = Join-Path $PSScriptRoot "..\models"
$out = Join-Path $dir "birefnet-lite.safetensors"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
if (Test-Path $out) { Write-Host "Already here: $out"; exit 0 }
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile $out
$size = (Get-Item $out).Length
if ($size -lt 80MB) { Remove-Item $out; throw "Downloaded file is too small ($size bytes)" }
Write-Host ("Saved {0} ({1:N0} MB)" -f $out, ($size / 1MB))
