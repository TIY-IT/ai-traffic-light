param([string]$Event, [string]$TargetStateDirectory)
$ErrorActionPreference = 'Stop'
try {
  if (-not [Console]::IsInputRedirected) { exit 0 }
  $payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
  $session = [string]$payload.session_id
  if (-not $session) { $session = [string]$payload.sessionId }
  if (-not $session) { exit 0 }
  $state = switch ($Event) {
    'UserPromptSubmit' { 'running' }
    'SessionStart' { 'running' }
    'PermissionRequest' { 'waiting' }
    'Stop' { 'stopped' }
    'StopFailure' { 'stopped' }
    'SessionEnd' { 'stopped' }
    default { '' }
  }
  if (-not $state) { exit 0 }
  $directory = if ($TargetStateDirectory) { $TargetStateDirectory } else { Join-Path $env:USERPROFILE '.workbuddy\wb-light\sessions' }
  New-Item -ItemType Directory -Force -Path $directory | Out-Null
  $hash = [Security.Cryptography.SHA256]::Create()
  try { $key = [BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($session))).Replace('-','').ToLowerInvariant() } finally { $hash.Dispose() }
  $target = Join-Path $directory "$key.json"
  $temporary = Join-Path $directory "$key.$PID.tmp"
  $value = @{ session_id = $session; state = $state; event = $Event; ts = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() / 1000.0 }
  [IO.File]::WriteAllText($temporary, ($value | ConvertTo-Json -Compress), (New-Object Text.UTF8Encoding($false)))
  Move-Item -LiteralPath $temporary -Destination $target -Force
} catch {
  # A notification failure must never interrupt the agent.
}
exit 0
