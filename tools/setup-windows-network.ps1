$ErrorActionPreference = "Stop"

$rules = @(
  @{ DisplayName = "AgentEye Host Hub 17891"; Port = 17891 },
  @{ DisplayName = "AgentEye Phone PWA 1421"; Port = 1421 },
  @{ DisplayName = "AgentEye Host Dev UI 1420"; Port = 1420 }
)

foreach ($rule in $rules) {
  $existing = Get-NetFirewallRule -DisplayName $rule.DisplayName -ErrorAction SilentlyContinue
  if ($existing) {
    Write-Host "Exists: $($rule.DisplayName)"
    continue
  }

  New-NetFirewallRule `
    -DisplayName $rule.DisplayName `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort $rule.Port `
    -Profile Private,Domain | Out-Null

  Write-Host "Added: $($rule.DisplayName)"
}

Write-Host "AgentEye Windows network rules are ready."
