---
name: status-light-doctor
description: 诊断与修复 AI 状态灯(七灯)的客户端接入问题。当用户提到状态灯/红绿灯不亮、豆包/扣子/TRAE/WorkBuddy 的灯没反应或乱跳、AI-Status-Light 安装/快捷方式/hooks 问题、灯挡住点击、悬浮窗找不到/空白/自己移动、团队面板/管理台收不到上报时使用。涵盖 WorkBuddy hooks、豆包三路探测、TRAE 双源、扣子调试端口、悬浮窗穿透与位置、团队管理台部署的完整诊断树与修复流程。
---

# AI 状态灯(七灯)诊断与修复

## 架构总览

状态灯程序(`AI-Status-Light-Recovery.exe` 或安装版 `AI-Status-Light.exe`)每 0.5 秒扫描
`~\.ai-traffic-light\sessions\<client>-*.json` 聚合出灯色。每个客户端的数据源不同:

| 客户端 | 数据源 | 需要专用快捷方式? |
|---|---|---|
| Codex | codex_local(~/.codex 数据库) | 否 |
| WorkBuddy | 官方 hooks → status_writer.ps1 → 会话文件 | 否(hooks 随配置生效) |
| ZCode | 本地 SQLite(~/.zcode/cli/db/db.sqlite) | 否 |
| Marvis | 本地会话库(只读) | 否 |
| 扣子 | CDP 127.0.0.1:19222 + React 探测 | **是**(普通启动灯看不见) |
| 豆包 | CDP 127.0.0.1:19224 + 三路信号合并 | **是** |
| TRAE | state.vscdb(结束红)+ CDP 19226 实时绿 | **是**(实时绿需要;结束红不需要) |

灯色语义:绿=识别到的会话在执行;红=结束/等待/异常(不保证结果正确);不亮=未运行/未接入/过期。

## 通用诊断顺序(任何灯不亮)

1. **灯本体在运行?** `Get-Process AI-Status-Light-Recovery` — 没运行则启动它。
2. **客户端在运行?** 灯位只显示运行中的客户端;没运行的客户端灯位自动隐藏(正常)。
3. **扣子/豆包/TRAE 是否从"(AI 状态灯)"快捷方式启动?** 普通方式启动 = 无调试端口 = 灯看不见。
   点一下对应快捷方式,弹窗选"是"会重启客户端并开启识别通道。
4. **会话文件有更新?** `~\.ai-traffic-light\sessions\` 里对应 `<client>-*.json` 的 mtime。
5. **hooks 处理器指向的 writer 文件存在?** WorkBuddy 的 handlers 指向
   `~\.ai-traffic-light\hooks\status_writer.ps1` — 该文件被删则 hooks 全部静默失败。

## 悬浮窗本体排障(窗口找不到 / 空白 / 自己动)

### 窗口"不见了"

- 进程在(`Get-Process AI-Status-Light-Recovery` 有结果)但屏幕上看不到 → 八成是窗口被拖到
  显示器边缘外。位置存在 `~\.ai-traffic-light\window-pos.json`(格式 `"x,y"` 物理坐标)。
  救急:`printf '1200,100' > ~/.ai-traffic-light/window-pos.json` 后重启状态灯。
- 2026-09-10 起启动恢复带**显示器钳制**(至少 80×80 物理像素在屏内),旧版才会启动即消失。
- 另注意:没有任何客户端在跑时悬浮窗自动隐藏(只留托盘)——这是设计行为,托盘菜单
  「显示悬浮灯」可召回。

### 窗口空白 / WebView 显示"无法访问此页面"

- 症状特征:窗口宽度停在初始 566、无任何内容、List Windows 宽度不随灯数变化 →
  **前端根本没加载**。
- 根因:用了 `cargo build --release` 直接构建。Tauri 缺 `custom-protocol` feature 时
  产物是 dev 模式二进制,运行时去连 devUrl(localhost:1420),没有 dev 服务器就是错误页。
- 正确构建:**必须** `cargo build --release --features custom-protocol`(Cargo.toml 已补
  `[features] custom-protocol = ["tauri/custom-protocol"]`);或走 `pnpm tauri build`。

### 窗口"自己动"

- 2026-09-10 起代码里**除启动恢复(带钳制)外不存在任何 set_position**;宽度变化只改
  大小不改位置,面板固定在灯右侧展开。若再现自动移位 = 新回归,按回归查。
- 点击穿透机制:前端上报可见灯壳/面板矩形,后端 25ms 轮询光标切换
  `set_ignore_cursor_events`——只有可见区域收鼠标,空白区点击直达下层。
  自查:悬停灯壳光标应呈 grab 手型(命中);点空白区后前台窗口应切到下层。
- 实现教训:`set_ignore_cursor_events` **只能在状态变化时调用**。曾因每 25ms 拍先
  false 再 true 翻转,瞬间恢复接收吞掉点击——状态机记 `Option<bool>`,变化才切换。

## 团队管理台(团队面板)专项

- 常驻部署(本机):计划任务 `AI-Status-Light-TeamServer`(ONLOGON)→
  `team-server-demo\Start-Team-Server-Hidden.vbs` → `ai-light-team-server.exe --port 8787`,
  隐藏窗口运行。**VBS 必须纯 ASCII**:含中文的 .vbs 存成 UTF-8 会被 wscript 按 ANSI 解析,
  首行赋值被坏字节污染报"缺少对象",还会把报错框挂到桌面。
- 数据(`server.db`)与密钥(`admin-token.txt`)按 **exe 所在目录**解析,与启动 CWD 无关;
  从别的目录启动不会"另建数据库",但统一从 team-server-demo 启动最稳。
- 管理页:`http://127.0.0.1:8787`;局域网 `http://<LAN-IP>:8787`(需防火墙入站规则
  `AI-Light-Team-Server-8787`,TCP 8787,已建)。密钥两种给法:页面密钥框粘贴后点「连接」
  (localStorage 记忆),或收藏 `http://<ip>:8787/?token=<密钥>` 直连(页面自动存并从地址栏移除)。
- 上报:客户端面板「团队」按钮启用,默认 10 秒一次;payload=activeClients(进程在跑的客户端)
  + sessions(client/title/state)。**不含会话正文**。面板里 client 显示英文 id(doubao/coze…)。
- 排障顺序:`netstat :8787` 有 LISTEN?→ 只有 TIME_WAIT 无 LISTEN = 服务死了但客户端还在
  重试,`schtasks /Run /TN "AI-Status-Light-TeamServer"` 拉起;→ `server.db` mtime 是否前进
  (前进=上报链路通);→ 客户端侧看面板「团队」状态行(401=密钥不对,连接拒绝=地址/防火墙)。
- 豆包/扣子要在面板出现会话,**必须从"(AI 状态灯)"快捷方式启动**(CDP 通道);普通启动只会
  出现在 activeClients 标签里,没有运行/停止状态。


## WorkBuddy 专项

### 注册(修复/重装 hooks)

```powershell
# 用交付包的 Configure-Hooks,指向稳定 writer 路径:
D:\3Dauto\ai-traffic-light\delivery\Configure-Hooks.ps1 -Client workbuddy `
  -WriterPath "$env:USERPROFILE\.ai-traffic-light\hooks\status_writer.ps1"
```

- 它会把 8 个事件(SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/PermissionRequest/Stop/StopFailure/SessionEnd)
  注册进 `~\.workbuddy\settings.json`,全部为**普通命令形式**。
- **注册后必须重启一次 WorkBuddy**(当前实例按旧配置执行;实测新条目在未重启的实例上不生效)。

### 已证实的坑(勿重蹈)

1. **EncodedCommand 形式的处理器 WorkBuddy 不执行**。普通命令形式正常触发,
   手动执行同样的 EncodedCommand 又正常——WorkBuddy 侧对命令形式的执行差异。
   注册/排查时一律用普通命令形式,不用 base64 编码形式。
2. **status_writer.ps1 绝不能无限等待 stdin**:WorkBuddy 触发 hooks 时不关闭管道,
   `[Console]::In.ReadToEnd()` 会永久阻塞 → 处理器卡死 → WorkBuddy 的 hook 队列
   (串行)整个堵死 → **应用所有操作卡住("使用不了")**。已修复:仅当
   `[Console]::IsInputRedirected` 时读取,且 ReadToEndAsync 硬性 800ms 上限。
3. 同一 settings.json 里兼容保留了旧项目的 python 处理器(wb_light_hook.py,写
   `~\.workbuddy\wb-light\sessions\`)——它与我们的处理器并行触发,互不影响,勿删。
4. writer 文件被删 = hooks 全部静默失败(无报错、无文件、灯暗)。清理安装目录前
   先确认 hooks 指向的路径。

### 验证

1. WorkBuddy 里发一条消息。
2. `~\.ai-traffic-light\sessions\workbuddy-*.json` 出现/更新(state=working)。
3. 灯绿;回复结束 → Stop 事件 → state=completed → 灯红(保持 24 小时)。

## 豆包专项

### 信号结构(三层合并,取最强,不短路)

1. 计数服务 `__messageTaskAssociationService__`(部分版本存在;**不跟踪普通聊天**,勿作为唯一信号);
2. DOM 标记 `[data-streaming="true"]`(生成中的回复气泡);
3. **通用网络钩子**(跨版本兜底):页面内一次性安装 fetch/XHR 钩子,统计流式响应节奏,
   "≥3 数据块、持续 >1.5 秒的待完成流" = 生成中。

已知:计数服务在同实例重启后可能不再计数普通聊天(0909 实测);
DOM 标记部分版本不翻转。网络钩子层为跨版本通用信号。

### 探测文件热更新

探测逻辑在安装目录的文本文件里(doubao-status.js / doubao-status.ps1),
程序每 3 秒重新读取——**修改后约 3 秒自动生效,无需重启状态灯或重编译**。
适配新豆包版本 = 改这个 js 文件放进安装目录。

### 诊断

生成一条回复的过程中运行 `Check-Doubao-Signal.ps1`(或双击 .cmd):
输出每页面的 svc/active/ended/streamingEls/net 状态,75 秒采样自动存
Check-Doubao-result.txt。判读:
- `svc=object` → 计数服务存在(生成中 active≥1);
- `streamingEls≥1` → DOM 标记在;
- 都没有但豆包在生成 → 该版本走未知的第三种通道,把输出发回来分析。

## TRAE 专项

双源:state.vscdb 的 solo 模式状态表(任务结束节点写入 status 1-10,
24 小时过期;**普通聊天不更新此表**) + CDP 19226 实时信号
(`__CHAT_SESSION_COLLECTOR__.sessions.size > 0` = 生成中)。
TRAE 的 ai-agent/database.db 是加密库,不可读。

## 扣子专项

CDP 19222 + React fiber 探测(读当前挂载会话的 sessionStatus)。
仅识别当前打开的会话;后台会话不保证。

## 交付与安装

- 最新交付包:`releases\AI-Status-Light-Beta-20260910-105951.zip`(**取代此前全部包**,
  含死区点击穿透、禁止自动移位、启动位置钳制、custom-protocol 构建修正;
  README.txt 内置"报错之后给 AI 助手"的即贴提示词,目标机器的 agent 照提示词即可自查)。
- 包内含**完整源代码**(source/ 目录,除编译产物),另附 TUTORIAL.md 与本技能文档,
  可在任何机器现场排查修复。
- 安装:解压 → 双击 Install.cmd → 装到 `%LOCALAPPDATA%\AIStatusLightBeta`,
  自动创建桌面快捷方式、配置 Codex/WorkBuddy hooks。
- 检测不到客户端时会提示输入其 exe 完整路径(只需一次,记住在 client-paths.json)。
- 检测范围:运行中进程 → 标准路径 → 开始菜单/桌面快捷方式扫描 → 注册表 → 手动输入。

## 修复脚本(随技能自带,scripts/ 目录)

| 脚本 | 用途 |
|---|---|
| `Configure-Hooks.ps1` | WorkBuddy/Codex hooks 注册与卸载(普通命令形式;`-Uninstall` 卸载) |
| `status_writer.ps1` | hooks 处理器调用的写入器(含 stdin 防卡死修复) |
| `Start-Doubao-Monitoring.ps1` | 豆包识别模式启动器(19224;自动探测安装路径,找不到会弹窗要路径) |
| `Start-Trae-Monitoring.ps1` | TRAE 识别模式启动器(19226) |
| `Start-Coze-Monitoring.ps1` | 扣子识别模式启动器(19222) |
| `Check-Doubao-Signal.ps1` | 豆包信号诊断(生成中运行,75 秒采样) |

以上脚本含中文,**必须保持 UTF-8 BOM**(无 BOM 时 PowerShell 5.1 按 ANSI 解析直接报错)。
修改后用 `[scriptblock]::Create(([IO.File]::ReadAllText(<file>)))` 验证语法。

## 修复后必测

1. WorkBuddy:重启 WorkBuddy → 发消息 → 灯绿 → 结束红。
2. 豆包:快捷方式启动 → 发消息 → 绿 → 红。
3. TRAE:快捷方式启动 → 跑任务 → 绿 → 红。
4. 扣子:快捷方式启动 → 发消息 → 绿 → 红。
5. 点击穿透:鼠标放灯位缝隙上点下层窗口,应能点中(灯不再挡点击)。
6. 窗口不自动移位:悬停展开/收起面板、灯数增减时,窗口 x,y 坐标不变;只有手动拖动才动。
7. 启动钳制:把 window-pos.json 写成屏幕外坐标(如 3000,100)重启,窗口应被拉回屏内可见处。

## 会话完成通知收不到 toast（2026-09-18 实战）

症状：notify 已部署、托盘「会话完成通知」已勾选，合成会话 working→completed 后无 toast。

排查序（按命中概率）：
1. 系统总开关（本次根因）：`reg query "HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\PushNotifications" //v ToastEnabled`，`0x0` = 全局静默丢弃一切 toast（连 PowerShell 测试 toast 都进不了平台）。改回：`reg add ... //t REG_DWORD //d 1 //f`。
2. WpnUserService 不热读该设置：改注册表后必须重启 `WpnUserService_<后缀>`（`Get-CimInstance Win32_Service -Filter "Name LIKE 'WpnUserService%'"` 找名字；Stop-Service 后不会自启，要手动 Start-Service）。重启后平台 `Setting` 才变 `Enabled`。
3. 平台侧验证：PowerShell 查 `[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('<AUMID>').Setting`，返回 0(Enabled) 才算通；测试 toast 用系统 AUMID `{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe`。
4. 落库验证（最硬证据）：查 `C:\Users\<用户>\AppData\Local\Microsoft\Windows\Notifications\wpndatabase.db` 的 `Notification` 表 `Type='toast'`。坑：`file:...?mode=ro` 只读连接读不到 WAL 最新行；拷贝 db+wal 到别处也可能赶不上写入——用普通连接对实时库 SELECT 最可靠（只 SELECT 无害）。
5. 应用 AUMID 注册友好名（否则通知来源显示原始 ID）：`reg add "HKCU\SOFTWARE\Classes\AppUserModelId\com.pcj1997.ai-traffic-light" //v DisplayName //d "AI 状态灯" //f`。

行为口径：通知只发"见过运行"的会话（prev=working → completed/waiting/error），一次结束只发一条；托盘开关持久化在 `~\.ai-traffic-light\config.json` 顶层 `notify_enabled`。

### 补充（2026-09-18 二次复发）：构建必须带 custom-protocol，打包脚本已内建
任何直接 `cargo build --release`（含 coder 子代理默认执行）都会产出 dev-mode 二进制：启动后 WebView 显示"无法访问此页面"。唯一正确构建命令（根工作区、虚拟 manifest，必须 -p 限定包并显式给特性）：
`cargo build --release --manifest-path Cargo.toml -p ai-traffic-light --features ai-traffic-light/custom-protocol`
部署后验证：`netstat -ano | grep <pid>` 中不应有任何 :1420（devUrl）连接；有即 dev-mode。Build-Delivery.ps1 已改为打包前自行执行上述构建，勿再手工拷 exe 进包。

### 通知点击跳转会话（2026-09-18）
tauri-plugin-notification 的 Rust builder 不支持点击回调；要点：send 直调其 Windows 后端 tauri-winrt-notification 0.7.3（Toast::new(app_id).on_activated(...)，app_id 必须 = app.config().identifier 才复用注册过的 AUMID 显示名；闭包需 Send+'static 用 owned String）。跳转实现 focus.rs：桌面客户端(coze/doubao/trae/marvis/workbuddy/codex)按进程名匹配可见顶层窗口；CLI 类(zcode 等)读 sessions/<id>.json 的 cwd 取目录名匹配终端窗口标题；SwitchToThisWindow+SetForegroundWindow 调前台。pid→进程名用项目已有 sysinfo，不给 winapi 加 psapi feature。
