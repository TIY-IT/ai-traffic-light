$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$log = Join-Path $PSScriptRoot 'Check-Doubao-result.txt'
$lines = New-Object System.Collections.Generic.List[string]
function Say($text) {
  Write-Output $text
  $lines.Add([string]$text)
}
Say '=== Doubao signal check ==='
Say 'Sampling for 75 seconds. SEND ONE MESSAGE IN DOUBAO NOW.'
$expr = 'JSON.stringify({href: location.href.slice(0,50), svc: typeof window.__messageTaskAssociationService__, active: (window.__messageTaskAssociationService__ && window.__messageTaskAssociationService__.getSnapshot) ? window.__messageTaskAssociationService__.getSnapshot().activeReplyCount : -1, ended: (window.__messageTaskAssociationService__ && window.__messageTaskAssociationService__.getSnapshot) ? window.__messageTaskAssociationService__.getSnapshot().endedReplyCount : -1, streamingEls: document.querySelectorAll("[data-streaming=true]").length, net: window.__AI_LIGHT_NET__ ? { hooked: window.__AI_LIGHT_NET__.hooked, activeStreams: window.__AI_LIGHT_NET__.streams.filter(function(s){return s.pending && s.chunks>=3}).length } : null})'
$deadline = (Get-Date).AddSeconds(75)
try {
  $targets = Invoke-RestMethod 'http://127.0.0.1:19224/json/list' -TimeoutSec 5
} catch {
  Say ('Port 19224 not reachable: ' + $_.Exception.Message)
  Say 'Doubao must be started via the (AI status light) shortcut first.'
  $lines | Add-Content -LiteralPath $log -Encoding utf8
  exit
}
Say ("port 19224 listening, targets: " + @($targets).Count)
while ((Get-Date) -lt $deadline) {
  foreach ($t in $targets) {
    if ($t.type -ne 'page') { continue }
    $socket = $null
    $cancel = New-Object System.Threading.CancellationTokenSource
    $cancel.CancelAfter(5000)
    try {
      $socket = New-Object System.Net.WebSockets.ClientWebSocket
      $null = $socket.ConnectAsync([Uri]$t.webSocketDebuggerUrl, $cancel.Token).GetAwaiter().GetResult()
      $request = @{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $expr; returnByValue = $true } } | ConvertTo-Json -Depth 5 -Compress
      $bytes = [Text.Encoding]::UTF8.GetBytes($request)
      $null = $socket.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $cancel.Token).GetAwaiter().GetResult()
      $buffer = New-Object byte[] 65536
      $stream = New-Object IO.MemoryStream
      do {
        $result = $socket.ReceiveAsync([ArraySegment[byte]]::new($buffer), $cancel.Token).GetAwaiter().GetResult()
        $stream.Write($buffer, 0, $result.Count)
      } while (!$result.EndOfMessage)
      $response = [Text.Encoding]::UTF8.GetString($stream.ToArray()) | ConvertFrom-Json
      $stream.Dispose()
      Say ("{0}  [{1}]  {2}" -f (Get-Date -Format 'HH:mm:ss.fff'), $t.url, $response.result.result.value)
    } catch {
      Say ("{0}  probe error: {1}" -f (Get-Date -Format 'HH:mm:ss.fff'), $_.Exception.Message)
    } finally {
      if ($socket) { $socket.Dispose() }
      $cancel.Dispose()
    }
  }
  Start-Sleep -Milliseconds 400
}
Say '=== done. Send Check-Doubao-result.txt back to support. ==='
$lines | Add-Content -LiteralPath $log -Encoding utf8
