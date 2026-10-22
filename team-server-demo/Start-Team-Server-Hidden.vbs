' Launch team server hidden (no console window). ASCII only: wscript reads
' .vbs as ANSI, so non-ASCII comments here would corrupt parsing.
' Data (server.db / admin-token.txt) resolve to the exe's own directory.
Set shell = CreateObject("WScript.Shell")
shell.CurrentDirectory = "D:\3Dauto\ai-traffic-light\team-server-demo"
shell.Run """D:\3Dauto\ai-traffic-light\team-server-demo\ai-light-team-server.exe"" --port 8787", 0, False
