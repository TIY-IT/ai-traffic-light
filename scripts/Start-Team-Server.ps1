param([int]$Port = 8787)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$exe=Join-Path $root 'target\release\ai-light-team-server.exe'
if(-not (Test-Path -LiteralPath $exe)){
  $exe=Join-Path $root 'releases\retained-apps-20261011\current\ai-light-team-server.exe'
}
if(-not (Test-Path -LiteralPath $exe)){
  Write-Output '未找到 target\release\ai-light-team-server.exe,请先执行:'
  Write-Output '  cargo build --release -p ai-light-team-server'
  Read-Host '按回车退出'
  exit 1
}
Write-Output "启动团队面板(端口 $Port),数据与 admin-token.txt 保存在 exe 同目录。"
& $exe --port $Port
Read-Host '服务已退出,按回车关闭窗口'
