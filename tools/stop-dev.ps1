$ports = @(1420, 1421, 17891)
$connections = Get-NetTCPConnection -LocalPort $ports -ErrorAction SilentlyContinue |
  Select-Object -ExpandProperty OwningProcess -Unique

foreach ($processId in $connections) {
  try {
    $process = Get-Process -Id $processId -ErrorAction Stop
    Write-Host "Stopping $($process.ProcessName) ($processId)"
    Stop-Process -Id $processId -Force -ErrorAction Stop
  } catch {
    Write-Host "Skip ${processId}: $($_.Exception.Message)"
  }
}
