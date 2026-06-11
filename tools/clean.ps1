$ErrorActionPreference = "Stop"

$root = Split-Path $PSScriptRoot -Parent
$tauriDir = Join-Path $root "apps\host\src-tauri"

Write-Host "AgentEye cleanup starting..."

if (Test-Path (Join-Path $tauriDir "Cargo.toml")) {
  Push-Location $tauriDir
  cargo clean
  Pop-Location
} else {
  Write-Host "Skip cargo clean: Cargo.toml not found"
}

$optionalDirs = @(
  (Join-Path $root "apps\host\node_modules\.vite"),
  (Join-Path $root "apps\phone\node_modules\.vite"),
  (Join-Path $root "agent_vision\history"),
  (Join-Path $root "agent_vision\dev_logs")
)

foreach ($dir in $optionalDirs) {
  if (Test-Path $dir) {
    Remove-Item $dir -Recurse -Force
    Write-Host "Removed: $dir"
  }
}

$total = (Get-ChildItem $root -Recurse -File -ErrorAction SilentlyContinue |
  Measure-Object -Property Length -Sum).Sum
Write-Host ("Done. Project size now about {0:N2} GB" -f ($total / 1GB))
