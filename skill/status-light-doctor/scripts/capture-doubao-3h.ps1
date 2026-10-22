$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$log = 'C:\Users\Administrator\.ai-traffic-light\doubao-capture.log'
$expr = 'JSON.stringify({href: location.href.slice(0,50), svc: typeof window.__messageTaskAssociationService__, active: (window.__messageTaskAssociationService__ && window.__messageTaskAssociationService__.getSnapshot) ? window.__messageTaskAssociationService__.getSnapshot().activeReplyCount : -1, ended: (window.__messageTaskAssociationService__ && window.__messageTaskAssociationService__.getSnapshot) ? window.__messageTaskAssociationService__.getSnapshot().endedReplyCount : -1, streamingTrue: document.querySelectorAll("[data-streaming=true]").length, netPending: window.__AI_LIGHT_NET__ ? window.__AI_LIGHT_NET__.streams.filter(function(s){return s.pending && s.chunks>=3}).length : -1})'
$deadline = (Get-Date).AddMinutes(180)
function Say($line) {
  Write-Output $line
  Add-Content -LiteralPath $log -Value $line -Encoding utf8
}
Say ("capture3 started " + (Get-Date -Format 'HH:mm:ss') + " (180 min window)")
while ((Get-Date) -lt $deadline) {
  try {
    $targets = Invoke-RestMethod 'http://127.0.0.1:19224/json/list' -TimeoutSec 5
    foreach ($t in $targets) {
      if ($t.type -ne 'page' -or $t.url -notlike 'doubao://doubao-chat*') { continue }
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
        Say ("{0}  {1}" -f (Get-Date -Format 'HH:mm:ss'), $response.result.result.value)
      } catch {
        Say ("{0}  probe error: {1}" -f (Get-Date -Format 'HH:mm:ss'), $_.Exception.Message)
      } finally {
        if ($socket) { $socket.Dispose() }
        $cancel.Dispose()
      }
    }
  } catch {
    Say ("{0}  list error: {1}" -f (Get-Date -Format 'HH:mm:ss'), $_.Exception.Message)
  }
  Start-Sleep -Milliseconds 500
}
Say 'capture3 ended'
