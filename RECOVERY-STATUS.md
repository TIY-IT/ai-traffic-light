# 本机恢复版

本文件覆盖早期文档中“Codex 必须使用 Hooks”的说明。

## 死区点击穿透 + 禁止自动移位（2026-09-10）

用户反馈:窗口隐形区域挡住下层窗口点击("有时候点击失败"),且窗口会自己移位。

- **点击穿透(矩形命中)**:前端把可见灯壳与展开面板卡片的矩形(逻辑坐标)经新 command
  `set_hit_regions` 报给后端;后端 25ms 轮询光标,仅落在矩形(±2px)内才
  `set_ignore_cursor_events(false)`,其余区域点击直达下层窗口。实测:灯壳上光标呈
  grab 手型(命中),点死区后前台从状态灯变为下层 TRAE(穿透生效)。
  0908 撤回穿透是因为当时与空白区拖动冲突;现拖动只靠灯壳 data-tauri-drag-region,冲突不存在。
- **禁止自动移位**:删除 resize_light_to_fit 的 anchor_right 平移分支与前端
  updatePanelPlacement 的 ±158px 换边平移(用户规则:窗口只能手动拖动)。面板固定在灯右侧展开,
  靠右屏时可能被屏幕边裁剪,拖动灯即可。全库仅剩启动恢复一处 set_position。
- **启动位置钳制**:恢复坐标时用 available_monitors 钳制,保证至少 80×80 物理像素落在某台
  显示器内(实测屏外坐标 3000,100 被拉回 1149,1021)——根治"拖到屏外后启动即消失"。
- **宽度含面板**:needWidth 现在取可见灯壳与展开面板栈的较大者,窗口 6 灯时 494(旧版 698)。
- 修复:轮询线程原先每拍先 false 再 true 翻转(80 次/秒,瞬时恢复接收可能吞点击),
  改为状态变化才调用。
- 构建:必须 `cargo build --release --features custom-protocol`。Cargo.toml 原本没有
  [features] 段,直接 cargo build 出的是 dev 模式二进制(运行时连 devUrl 的 1420 端口,
  WebView 显示"无法访问此页面",前端完全不加载)。已补 custom-protocol feature。
- cargo test 51 项通过;交付包未重打(下次交付需重打)。

## 团队管理台常驻化(2026-09-10)

- 计划任务 `AI-StatusLight-TeamServer`(ONLOGON)经
  `team-server-demo\Start-Team-Server-Hidden.vbs` 隐藏启动
  `team-server-demo\ai-light-team-server.exe --port 8787`;数据库路径按 exe 目录解析,CWD 无关。
- 坑:含中文的 .vbs 保存为 UTF-8(带 BOM)会被 wscript 按 ANSI 解析,
  首行 `Set shell=...` 被坏字节污染报"缺少对象",且报错对话框挂在桌面——VBS 一律纯 ASCII。
- 防火墙入站规则 `AI-Light-Team-Server-8787`(TCP 8787)放行局域网访问。
- 管理页 http://192.168.31.229:8787(本机 127.0.0.1),密钥 team-server-demo\admin-token.txt。

使用教程见根目录 `TUTORIAL.md`（含 Windows 排障与 macOS 现状）。2026-09-07 顺带修复
presence.rs 进程名只认 `.exe` 后缀的问题（macOS 进程名无后缀会导致悬浮窗永不显示），
两种形式现在都识别，27 项后端测试通过。

## TRAE 与豆包接入（2026-09-07）

面板从 5 灯扩为 7 灯（新增 TRAE、豆包），数据均只读元数据、不碰聊天正文：

- **TRAE（TRAE SOLO CN）**：新适配器 trae.rs 只读
  `%APPDATA%\TRAE SOLO CN\User\globalStorage\state.vscdb` 的 solo 模式状态表
  （key 形如 `solo-lite-mode-state-map-<uid>`），取 Work/Code/Design 等模式各自当前会话的
  名称/状态/时间。status 枚举从客户端前端包恢复：0 未知、1 排队、2 创建中、3 运行中、
  4 停止、5 完成、6 失败、7 冻结、8 等待、9 回滚中、10 停止中。>24h 的旧会话自动过期。
  TRAE 自带的 ai-agent/database.db 是加密库，不在读取范围。
- **豆包**：新适配器 doubao.rs 走 CDP `127.0.0.1:19224`（与扣子 19222 同一套显式启用模式），
  对 `doubao://doubao-chat/chat` 页面求值 `__messageTaskAssociationService__.getSnapshot()`
  的两个计数：activeReplyCount>0→绿（生成中），endedReplyCount>0→红（已结束），
  都为 0→灰（已连接）。桌面「豆包（AI 状态灯）」快捷方式启动；普通模式启动的豆包不可见。
- 验收（2026-09-07 更新）：悬浮窗新增点击穿透(2026-09-08):窗口固定 566×226,透明/空白区域此前会挡住
  下层窗口点击;现由前端上报可见灯位矩形、后端 25ms 轮询光标切换
  set_ignore_cursor_events,只有灯位±2px 接收鼠标。cargo test 40 项通过；7 灯真实 UI 渲染正确；TRAE 的 3 条 5 天前
  已完成会话被过期规则正确丢弃（灯灭）。豆包实测闭环：观察器 14:38 采到真实生成
  （active=1 约 5 秒，绿灯），结束后灯保持红灯（状态记忆生效）；TRAE 生成中的 DOM 标记
  （停止按钮、--streaming 容器）已在真实生成窗口抓取验证，绿→红的最终目视确认待下次任务。
  另发现：豆包存在多个安装版本，AppData 版聊天页懒加载服务对象且无该服务时回退
  data-streaming DOM 标记；适配器表达式已兼容两种形态。采集未命中保留上一状态 30 秒。
  全程未向 TRAE/豆包发送消息、未消耗额度。
- 交付包同步重打：`AI-Status-Light-Beta-20260907-143345.zip`，见 DELIVERY-STATUS.md。

## CDP 客户端统一到三级降级结构（2026-09-07 晚）

扣子/豆包/TRAE 三个 CDP 客户端的探测表达式统一为同一结构：
精确信号优先（各版本内部对象/计数服务）→ DOM 标记 → **通用网络钩子兜底**
（页面内一次性安装 fetch/XHR 钩子，凡“≥3 数据块、持续 >1.5 秒”的流式响应 = 生成中，
与内部实现无关，跨版本通用）。豆包的通用层已在真实生成中验证（14:38 active=1）；
扣子的 React 精确探测在本机无回归（红灯正常），通用层为其未来版本兜底；
TRAE 实时信号扩为三重（收集器 + DOM + 网络钩子）。coze.rs 接受 live 兜底行，
cargo test 40 项通过。最终交付包 163213。局限：若某版本改用 WebSocket 流式传输
则需加对应通道（诊断脚本可判断）。

- Codex：新的独立只读实现读取本机任务生命周期事件，不依赖 Hooks，不修改信任设置。只认匹配 turn_id 的 task_complete / turn_aborted；item_completed 和普通回答不代表任务结束。按父子关系汇总，子代理仍执行时不能显示完成。
- ZCode：递归汇总子代理，包括多层子代理；排除前一轮任务遗留子代理，无法确认的子代理状态不判完成。
- 扣子：保留本机 CDP 当前会话适配。后台所有会话、真实生成到结束的完整验收仍未完成。
- 保留 × 收起、≡ 展开和 Esc；本机真实窗口测试通过。
- WorkBuddy：本机恢复启动带 --legacy-local-workbuddy，复用用户机器上已有的采集器，不把其代码复制或打包。此模式不是可移植的分发承诺。
- 旧版 EXE 保留；现有 ZIP 没有本轮修复，不应以这些 ZIP 宣称已修复子代理问题。

验收：Codex 本条真实任务在新版窗口亮绿灯；扣子真实停止状态亮红灯；面板开关测试通过。任务级结束、不同 turn_id 结束、多层子代理、子代理未知等有回归测试。尚需真实并发子代理生命周期实测，不能宣称所有场景已验证。

补充验收：26 项后端测试全部通过。新增只读回放实际 Codex 父子任务历史（4 次子代理状态变化），以及 ZCode 完整 SQLite 读取到父任务汇总状态的集成测试。日志半行追加、轮转、父结束子运行、子结束父运行、无首条消息子代理、未知状态均已覆盖。这里的历史回放不等同于新建真实并发任务；未发送测试消息、未消耗 AI 额度。

本机当前程序：releases/AI-Status-Light-Recovery.exe，SHA256 9FB8E4B02D5799BD082282EE385130883C1F64AC752F2EB22B4B64886A9F69DC；桌面“AI 状态灯（精致版）”指向此版本，带本机 WorkBuddy 兼容参数。指示灯自身调试端口已关闭，旧版文件和旧快捷方式备份仍保留。

## WorkBuddy 接入修复（2026-09-07）

症状：WorkBuddy 运行中但灯不亮。原因有两个：

1. 本机 WorkBuddy 的 hooks（`~/.workbuddy/settings.json`）还指向旧项目
   `D:/3Dauto/workbuddy-status-light/wb_light_hook.py`，状态写到
   `~/.workbuddy/wb-light/sessions/`；而本程序只读 `~/.ai-traffic-light/sessions/` 下的
   `workbuddy-*.json`。`--legacy-local-workbuddy` 依赖的 `%LOCALAPPDATA%\AIStatusLight`
   采集器在本机并不存在，所以两条数据链路都是空的。
2. delivery/Configure-Hooks.ps1 给 hook 注册写了 `timeout=3`。WorkBuddy 对该字段按毫秒
   解释（老 hook 无此字段时日志显示默认 60000ms），3 毫秒会被立即超时。已去掉该字段，
   让 WorkBuddy 使用其默认值；scripts/Test-Delivery.ps1 回归通过。

修复：把 hooks/status_writer.ps1 安装到 `~/.ai-traffic-light/hooks/`，运行
`delivery/Configure-Hooks.ps1 -Client workbuddy -WriterPath ...` 把 8 个事件
（SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/PermissionRequest/Stop/StopFailure/SessionEnd）
注册为 EncodedCommand 形式；旧 Python hook 保留共存，互不影响。

验收：用 settings.json 里真实注册的命令模拟 WorkBuddy 调用（stdin 传 payload），
UserPromptSubmit→绿灯“1 项运行”、Stop→红灯“1 项待处理”、SessionEnd→灯灭“暂无状态”，
全程截图目视确认；测试会话已通过 SessionEnd 清除，未向 WorkBuddy 发送任何消息、未消耗额度。
补充：11:53 用户真实使用 WorkBuddy 时新 hooks 已自动生效（无需重启客户端），
绿灯→红灯在真实任务上复现。

## 扣子状态灯启动入口修复（2026-09-07）

症状：找不到扣子状态灯的启动入口。原因：桌面“扣子（AI 状态灯）”快捷方式指向
`releases\Start-Coze-Monitoring.ps1`，但该文件没有随 EXE 一起放在 releases 目录，点击无反应。

修复：
- scripts/Start-Coze-Monitoring.ps1 两处调整：同目录存在 AI-Status-Light-Recovery.exe 时优先启动它
  （交付包只有 AI-Status-Light.exe，行为不变）；状态灯进程去重按脚本实际 adjacent 的 EXE 文件名匹配
  （原脚本报写死 AI-Status-Light.exe，会在本机拉起第二个灯实例）。
- 该脚本已复制到 releases/，与桌面快捷方式指向一致。注意 .ps1 含中文，必须保持 UTF-8 BOM
  （无 BOM 时 PowerShell 5.1 按 ANSI 解析直接报语法错误）。

验收：点击流程实测——弹窗提示重启扣子，确认后扣子带
`--remote-debugging-address=127.0.0.1 --remote-debugging-port=19222` 重启且仅监听本机回环，
悬浮灯扣子格亮红灯“已停/待处理”（当前会话）。重复点击脚本会复用已开调试的扣子，不再重启。
日常使用：桌面“扣子（AI 状态灯）”= 带状态识别的扣子启动方式；普通“扣子”图标启动的实例灯不可见，
需再用快捷方式切换。

## 2026-09-18 会话完成通知 + 面板徽章上线与 toast 排障

- 交付：`notify.rs`（见过运行才通知，托盘开关持久化 `notify_enabled`）；面板会话条目加状态徽章（运行中/已结束等）、运行中优先排序；`publishHitRegions` 精确点击区。
- 构建：root workspace `cargo build --release`（13:14:51），部署 `releases/AI-Status-Light-Recovery.exe`（13:16:25），单实例已验证。
- toast 排障：收不到通知根因是系统 `ToastEnabled=0`（全局关闭，来源不明）→ 改回 1 → 重启 `WpnUserService_d703c` 生效。验证：`wpndatabase.db` `Notification` 表出现 `AI 状态灯 · ZCode会话已结束` toast（普通连接直查实时库，只读 URI 读不到 WAL 新行）。
- 系统侧改动（告知用户）：`HKCU PushNotifications\ToastEnabled` 0→1；注册 `AppUserModelId\com.pcj1997.ai-traffic-light DisplayName="AI 状态灯"`。
- 清理：sessions 目录历史测试残留（*.tmp.*、enc-cmd-test 等 5 个旧测试 json）已删；合成测试文件已删。

## 2026-09-18 14:06 通知"看不见"根因 + 外观/提醒改造

- 根因：系统 toast 正常落库（13:34-13:36 共 4 条），但用户桌面处于全屏应用状态（SHQueryUserNotificationState=5），Windows 全屏静音规则只入通知中心不弹横幅 → 治本方案为应用内提醒：红灯 attn 闪烁 6s + WebAudio 提示音（受托盘开关+AudioContext state 双重门控）+ 面板「刚结束 N」角标。
- 红灯语义改新鲜度：ended 且 updated_at ≤15min 才亮红，陈旧已停灯全暗；「清空已停」按钮在有 ended 会话时显示。
- 通知冷却：同会话 90s 内不重发（cooldown_ok 纯函数+2 测试，52 passed）。
- 新包 20260918-140602（见 DELIVERY-STATUS）。

## 2026-09-18 14:13 custom-protocol 坑复发与根治

- coder 按 `cargo build --release` 产出 dev-mode 二进制（部署后 WebView 显示"无法访问此页面"），二次复发。
- 修复：`cargo build --release --manifest-path Cargo.toml -p ai-traffic-light --features ai-traffic-light/custom-protocol`；部署后用 `netstat -ano | grep <pid>` 验证无 :1420 连接。
- 根治：Build-Delivery.ps1 打包流水线内自行构建并校验退出码，不再拷贝现存 exe；两个当日旧 zip 已删除。
- 当前实例 PID 61588（14:12 构建）。

## 2026-09-18 14:34 通知点击跳转

- toast 激活：notify.rs send() 直用 tauri-winrt-notification 0.7.3（已在依赖树，插件 builder 无 on_activated），app_id 复用 config.identifier 保持 AUMID 显示名一致。
- 窗口聚焦：新增 focus.rs，纯函数 pick_window + 3 测试；pid→镜像名复用 sysinfo；SwitchToThisWindow+SetForegroundWindow；id 字符集校验防路径逃逸。
- 新命令 focus_session(client,id)，面板会话行可点击（hover 高亮）。55 tests passed。
