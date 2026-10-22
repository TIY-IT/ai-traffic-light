# Windows 本地整合版

基于 pcj1997/ai-traffic-light 的现有界面，保留上游 MIT 许可证。
本地增加 Codex 日志和 WorkBuddy 状态文件的只读桥接。

- 绿灯：至少一个已识别会话正在运行。
- 红灯：已识别会话已结束，或等待用户处理；不表示结果必然成功。
- 全灭：连接中、没有活动记录或状态未知，不冒充成功。
- 鼠标移入灯壳显示按 Codex / WorkBuddy 分组的状态；拖动灯壳移动。
- 从系统托盘菜单退出。没有自动安装或替换任何 AI Hooks。
- 已接入 Marvis：复用其安装目录中的 Python，以 SQLite 只读模式读取最近使用的用户数据库中的会话标题、状态及事件时间。只显示最近 24 小时记录；进行中超过 120 秒没有事件则显示未知，不冒充结束。客户端未启动时需启动 Marvis 后重开灯。此接口非公开稳定 API，升级后需复测。
- 不接入 Coze / ima，不消耗模型额度，不上传会话内容。

采样间隔 800 毫秒，前端刷新 500 毫秒，实际延迟还受客户端写日志影响。
本机生成的缓存位于 `%LOCALAPPDATA%\AIStatusLight`。
Codex 的日志格式并非稳定公共 API，客户端升级后需复测。
WorkBuddy 需预先配置兼容的状态 Hooks；单独复制 EXE 不会替老板配置 Hooks。

构建：`pnpm install --frozen-lockfile`、`pnpm build`、`pnpm tauri build --no-bundle`。
`.cargo/config.toml` 仅切换依赖索引镜像，不关闭 TLS 校验。
如镜像下载超时，scripts/fetch-crates.mjs 可从官方 CDN 下载 Cargo.lock 中的归档并校验 SHA-256。

## 团队面板（v0.2.0 起，内网可选）

仓库是 Cargo workspace：`src-tauri`（悬浮灯客户端）+ `server`（团队面板后端，单文件 exe）。
客户端上报为 opt-in（默认关），只上传成员名、机器名、在用软件、会话状态与项目名，
不含会话内容；配置存 `~/.ai-traffic-light/config.json` 的 `team` 字段。
后端启动方式与成员配置见 TUTORIAL.md 第九节；管理页内嵌在 server exe 里（`GET /`）。

此版本目前为本机适配验收版。嵌入的 status-monitor.ps1 复用了本地
codex-light-upstream 的采集代码；对外分发前还需核实该部分代码的授权，
并完成目标机器的 Hooks / WebView2 环境验收。
