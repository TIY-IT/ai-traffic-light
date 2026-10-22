# Windows 内测交付记录（2026-09-04）

交付物：releases/AI-Status-Light-Beta-20260904-153243.zip（22,284,547 字节）。
SHA-256：225F54C5608C216CDF4E0C8EBE6E0EA32984F216A2A3ADC63629FE2308A45648。

## 替代上述包的新交付（2026-09-10，含死区点击穿透/禁止自动移位）

交付物：releases/AI-Status-Light-Beta-20260910-105951.zip（23,395,355 字节），**取代此前全部包**
(同日早些的 105150 包缺 README 增补,作废)。
SHA-256：8D41D59CD41A3E42EA65CFA1BA2FFBE1E8CEAD749EFA2C524CA4F94F7AD9D1B3。
相对 0909 215240 包的新增：
1. 悬浮窗死区点击穿透:前端上报可见灯壳/面板矩形,后端 25ms 轮询切换
   set_ignore_cursor_events,空白区点击直达下层窗口(实测验证);拖动只靠灯壳,与穿透不再冲突。
2. 禁止窗口自动移位:删除 resize 锚边平移与面板 ±158px 换边平移;面板固定右侧展开。
3. 启动位置钳制:恢复坐标保证至少 80×80 物理像素落在某台显示器内,根治"启动即找不到窗口"。
4. 窗口宽度紧贴内容(6 灯 494px,旧 698px)。
5. 构建修正:Cargo.toml 补 custom-protocol feature——否则 cargo build --release 产物是
   dev 模式(运行时连 devUrl,WebView 显示"无法访问此页面")。
6. README.txt 增补"报错之后:把提示词发给 AI 助手"(通用排查+6 个场景的即贴提示词),
   并修正过时的"未附完整源码"说明(包内 source/ 即完整源码)。
cargo test 51 项通过;Test-Package(本 ZIP)通过:隔离 profile 安装/重装/卸载无损、exe 哈希一致。
团队面板(可选组件)不在包内,单独部署:见 RECOVERY-STATUS.md 2026-09-10 团队管理台常驻化。

## 替代上述包的新交付（2026-09-07 晚，含 TRAE/豆包）

交付物：releases/AI-Status-Light-Beta-20260909-215240.zip（此为最终交付包，
取代此前全部 Beta 包）。SHA-256：1F58C336F62A01EDBEFC0EDCE381511A8161F4CEEE7D32F9CD3EA3D4B3AD7034。
0909 145911 相对 133405 的新增:悬浮窗位置持久化(拖动后存盘、重启恢复)、窗口宽度自动
收缩紧贴可见灯位(消除透明死区,修复灯挡住下层窗口点击的问题;穿透方案撤回——与拖动冲突)、
WorkBuddy 普通命令 hooks 注册(详见上方 WorkBuddy 专项)。
相对更早版本的全部修复:
1. WorkBuddy hooks 关键修复:实测 WorkBuddy 不执行 -EncodedCommand 形式的处理器
   (普通命令形式正常触发;手动执行 EncodedCommand 又正常——WorkBuddy 侧执行差异),
   workbuddy 的 8 个事件注册全部改为普通命令形式,指向稳定 writer 路径
   ~/.ai-traffic-light/hooks/status_writer.ps1;交付包 Configure-Hooks.ps1 同步改为普通命令
   注册(老板机上也同样生效),旧格式清理逻辑兼容两种形式。
2. status_writer.ps1 补 stdin 有界读取(IsInputRedirected + 800ms 上限):WorkBuddy 触发
   hooks 时不关闭管道,旧版 ReadToEnd 会永久阻塞——0909 11:08 起卡死的处理器曾把
   WorkBuddy 的 hook 队列整个堵死,导致应用操作全部卡住(即“使用不了”事故)。已修复:
   处理器 0.84 秒内必定返回,不再可能卡死;卡死进程已清理。
3. 豆包探测改为三路信号合并取最强(网络钩子/DOM 流式标记/计数服务),原表达式被
   “消息任务服务”计数短路——该服务不跟踪普通聊天回复,导致豆包灯生成中不变绿;
   已实测验证修复生效(用户确认“现在可以了”)。
4. 远程可维护性:三个 CDP 客户端的探测表达式以文本文件随包交付(doubao/trae/coze-status.js
   与 .ps1,位于安装目录),程序每次轮询重新读取,修改后约 3 秒自动生效,无需重启或重编译;
   内置副本为缺省兜底。若某版本改用 WebSocket 流式传输则需加对应通道(诊断脚本可判断)。
取代此前全部 Beta 包）。SHA-256：见本目录 111351 包(以实际文件为准——status_writer.ps1 补了 stdin 有界读取防卡死修复)。
（152107 之后的修复：豆包探测改为三路信号合并取最强——原表达式被“消息任务服务”计数
短路，而该服务不跟踪普通聊天回复，导致豆包灯生成中不变绿；已实测验证修复生效。）
相对更早版本的关键修复：
1. Install.ps1/Uninstall.ps1 补 UTF-8 BOM（缺 BOM 时中文快捷方式名解析损坏，
   安装中止于扣子快捷方式，豆包/TRAE 快捷方式不创建——已实测复现并修复）。
2. 扣子/豆包/TRAE 检测升级：运行中进程 → 标准路径 → 开始菜单/桌面快捷方式扫描 →
   注册表卸载项（中英文产品名）→ 命令窗口手动输入路径兜底；完成提示同时提及三个客户端。
3. 包内附 Check-Doubao-Signal.ps1 与双击入口 Check-Doubao-Signal.cmd
   （双击后采样 75 秒，期间在豆包发一条消息，结果自动存为 Check-Doubao-result.txt 发回）
   与更新版 README。
安装脚本关键修复：delivery/Install.ps1、Uninstall.ps1 含中文快捷方式名但缺 UTF-8 BOM，
PowerShell 5.1 按 ANSI 解析导致创建快捷方式时报错终止（实测复现：安装中止于扣子快捷方式，
豆包/TRAE 快捷方式不会创建）。已加 BOM 并用解压目录完整安装实测：四个快捷方式全部创建，
Hooks 配置成功，Test-Package 通过。其余内容同前述最终构建（7 灯版，TRAE CDP 实时 +
模式状态表双源，豆包多页聚合 + 状态记忆，采集超时 5 秒、未命中保留 30 秒）。
悬浮窗点击穿透:窗口内只有可见灯位矩形(±2px)接收鼠标,其余透明区域点击直达下层窗口,
由前端上报灯位区域、后端 25ms 轮询光标位置切换实现。

相对 09-04 包的修复与新增：
- delivery/Configure-Hooks.ps1 去掉 hook 注册里的 `timeout=3`（WorkBuddy 按毫秒解释会立即超时，
  老板机 WorkBuddy 灯会因此不亮），改用其 60 秒默认值。
- scripts/Start-Coze-Monitoring.ps1：状态灯进程去重按实际 adjacent 的 EXE 文件名匹配；
  存在 Recovery 版时优先启动。含中文的 .ps1 保持 UTF-8 BOM。
- 新增 TRAE 灯（CDP 19226 实时 + state.vscdb 模式状态表）与豆包灯（CDP 19224 回复计数），
  Install.ps1 增加两个桌面快捷方式块与脚本复制。

验收：cargo test 36 项通过；Test-Delivery、Test-Package（对本 ZIP）通过；
本机真实 UI 验收 7 灯渲染正确；TRAE/豆包采集器单发实测均返回 connected:true。
豆包/TRAE 的绿→红需用户下次真实任务复现（全程未向客户端发送消息、未消耗额度）。

## 当天早些时候的交付（2026-09-07 12:50，已被上面的包取代）

交付物：releases/AI-Status-Light-Beta-20260907-125024.zip（22,318,766 字节）。
SHA-256：8FD02A15B5107E7BD03E223C7E355BF88B82B04F2E724119258576CB050354BA。
EXE 与本机已验证的 AI-Status-Light-Recovery.exe 字节一致（9FB8→0C2A 版本，2026-09-06 构建）。

相对 09-04 包的修复：
- delivery/Configure-Hooks.ps1 去掉 hook 注册里的 `timeout=3`（WorkBuddy 按毫秒解释会立即超时，
  老板机 WorkBuddy 灯会因此不亮），改用其 60 秒默认值。
- scripts/Start-Coze-Monitoring.ps1：状态灯进程去重按实际 adjacent 文件名匹配；
  存在 Recovery 版时优先启动。含中文的 .ps1 保持 UTF-8 BOM。

验收证据：
- cargo test：27 项通过（新增 macOS 进程名回归）。
- scripts/Test-Delivery.ps1：配置合并、重复安装、卸载保留其他配置、四种 Hook 状态及缺失会话 ID 拒绝通过。
- scripts/Test-Package.ps1 对新 ZIP：隔离安装/重装/卸载、EXE 哈希、无旧采集器与用户数据库，PASS。
- 包内 Configure-Hooks.ps1 已确认不含 timeout=3。
- 功能实测（本机）：WorkBuddy 绿/红/灭、扣子 CDP 红灯、Codex/ZCode 真实状态均验证通过，见 RECOVERY-STATUS.md。

尚未声称完成的范围：老板电脑实机测试、代码签名、所有客户端版本兼容性。
Codex 首次 Hooks 信任必须使用者自行审查；没有修改信任数据库或绕过安全确认。

---

以下为 2026-09-04 首次交付的历史记录。

此记录及包内 README.txt 替代早期 LOCAL-EDITION.md 的采集方式说明。
交付版已移除 status-monitor.ps1 及其嵌入引用；Codex/WorkBuddy 使用 MIT 上游修改版 Hooks writer，Marvis/ZCode 使用本地只读数据库适配。
源码中的 delivery/Configure-WorkBuddy.ps1 和 workbuddy-portable-hook.ps1 为早期草稿，不进入包；实际使用 Configure-Hooks.ps1。

验收证据：
- pnpm tauri build --no-bundle -- --offline 成功。
- cargo test：17 项通过。
- scripts/test-client-lights.cjs：四客户端状态隔离、显隐与未知状态通过。
- scripts/Test-Delivery.ps1：配置合并、重复安装、卸载保留其他配置、四种 Hook 状态及缺失会话 ID 拒绝通过。
- scripts/Test-Package.ps1：最终 ZIP 解压、隔离用户目录安装/重装/卸载、EXE 哈希、排除旧采集器与用户数据库通过。
- 最终包 EXE 启动，窗口有句柄且 Responding=true；测试进程已停止，原运行版本保留。

尚未声称完成的范围：老板电脑实机测试、代码签名、桌面截图目视验收、所有客户端版本兼容性。
Codex 首次 Hooks 信任必须使用者自行审查；没有修改信任数据库或绕过安全确认。
原运行版本未自动迁移，避免用户未审查 Hooks 时中断已有状态显示。
## 2026-09-18 新包：会话完成通知 + 面板徽章版

- 包：`releases/AI-Status-Light-Beta-20260918-134118.zip`
- SHA-256：`BFDF57D91472D228F669CF279DB2947FB1F3BAED39BFBDC8F214D4B8706B7062`
- 相对 20260910 包新增：会话完成系统通知（托盘开关+持久化）、面板会话状态徽章+运行中优先、精确点击区、单实例保护；skill 含 toast 排障经验。
- 打包修复：license-notices 离线元数据失败（新依赖未缓存）→ `cargo fetch` 补齐后通过；Test-Package 自检 PASS。

## 2026-09-18 14:06 二次打包：外观瘦身 + 全屏可靠提醒版

- 包：`releases/AI-Status-Light-Beta-20260918-140602.zip`（替代同日 13:41 包）
- 改动：灯体 72→56px 缩小柔化；红灯只亮"15 分钟内刚结束"；面板单行会话行+右侧状态小字；「清空已停」按钮；同会话通知 90 秒冷却；新增不依赖系统弹窗的结束提醒（红灯闪烁+提示音+面板「刚结束」角标）。
- 背景：用户全屏时 Windows 静默通知横幅（只进通知中心），故加应用内提醒通道。
- 自检：Test-Package PASS。运行实例 PID 14968。

### 作废说明（同日）
- `20260918-134118`（内容尚可但已被取代）与 `20260918-140602`（**内含 dev-mode 坏 exe**，打包时拷贝了缺 custom-protocol 特性的构建产物）两个 zip 已删除。
- **唯一有效包：`releases/AI-Status-Light-Beta-20260918-141351.zip`**，SHA-256 `8F3A554C8A2F52676CAA84F97DE5EB9928C96A5C96115A7A023024DCD9B76AD1`。
- 根治：Build-Delivery.ps1 现在打包前**自行构建**（`cargo build --release -p ai-traffic-light --features ai-traffic-light/custom-protocol`），不再信任目录里的现存 exe。

## 2026-09-18 14:34 三次打包：通知点击跳转会话版（当前唯一有效包）

- 包：`releases/AI-Status-Light-Beta-20260918-143427.zip`（141351 已删除作废）
- SHA-256：`216EADDCDECBB84DF677D1140F13A5825D002259D8836D396C3BB4289709CE23`
- 新增：点击"会话已结束"通知或面板会话行 → focus.rs 把对应客户端窗口调前台（桌面客户端按进程名 Coze.exe/Doubao.exe/Trae.exe/Marvis.exe/WorkBuddy.exe/Codex.exe；CLI 类按会话 cwd 目录名匹配终端标题）；toast 改直调 tauri-winrt-notification 以支持 on_activated。
- 自检 PASS；运行实例 PID 72336；合成验证新发送路径 toast 正常落库。

## 2026-09-18 14:4x 四次打包：面板排版修复版（当前唯一有效包）

- 包：`releases/AI-Status-Light-Beta-20260918-144036.zip（SHA-256 A8FCF969…B113047）`（以 releases 目录最新 zip 为准，前一个 143427 已删除）
- 改动：面板 150→190px；头部单行（状态文案不再被挤成竖排）；「刚结束 N」角标挪到第二行右侧与"会话数"同行；会话行/徽章收紧；「清空已停」改紧凑文本钮。纯 HTML/CSS，无逻辑改动。
