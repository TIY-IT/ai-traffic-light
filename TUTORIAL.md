# AI 状态灯（精致版）使用教程

> 更新：2026-09-07。本教程描述当前源码与 Windows 本机已验证的实际行为；
> macOS 部分单独列在第八节，未在真实 Mac 上验证过的地方都明确标注。

---

## 一、这是什么

一个悬浮在屏幕上的红绿灯 + 系统托盘图标，实时显示各个 AI 客户端的会话状态：

- **绿灯**：至少一个已识别的会话正在运行（AI 在思考、在调工具）。
- **红灯**：会话已结束或正在等待你处理（**不代表结果一定成功**）。
- **灯灭（灰）**：客户端没运行、没接入，或状态未知——不冒充成功。

鼠标移到灯壳上会展开面板，按客户端分组显示每个会话的状态；面板可拖动；
托盘菜单里有「显示悬浮灯 / 开机自启动 / 退出」。

**隐私原则**：只读本机已有的状态信号（Hook 事件、本地数据库、本机回环调试端口），
不发消息、不消耗模型额度、不上传任何会话内容。

---

## 二、Windows 安装

### 方式 A：交付 ZIP（给老板机 / 干净机器）

1. 解压 `AI-Status-Light-Beta-*.zip` 到任意位置。
2. 双击 **Install.cmd**。程序安装到 `%LOCALAPPDATA%\AIStatusLightBeta`，
   并自动完成：桌面快捷方式、开机自启、Codex 与 WorkBuddy 的 Hooks 配置。
3. 如果机器没装 WebView2，安装脚本会提示去微软官网装一下（Win11 一般自带）。
4. Codex 第一次跑任务时会弹出「是否信任 Hooks」，需要你本人确认——程序不会替你跳过这个安全确认。

### 方式 B：本机恢复版（当前这台开发机）

- 状态灯本体：`D:\3Dauto\ai-traffic-light\releases\AI-Status-Light-Recovery.exe`
  （桌面「AI 状态灯（精致版）」快捷方式已指向它）。
- 扣子启动脚本：同目录 `Start-Coze-Monitoring.ps1`（桌面「扣子（AI 状态灯）」快捷方式已指向它）。

---

## 三、七个客户端怎么接入（Windows）

面板上一共 7 个灯：**Codex / WorkBuddy / Marvis / ZCode / 扣子 / TRAE / 豆包**。
下面每个都给出：要做什么、怎么验证。

### 1. Codex ✅ 无需配置

恢复版直接只读 Codex 的本地任务数据库（`~/.codex/state_5.sqlite`），**不需要装 Hooks、
不改信任设置**。启动 Codex 跑任务，灯即绿；结束转红。子代理还在跑时不会误报完成。

### 2. WorkBuddy ✅ 一次性配置 Hooks（2026-09-07 已配好）

- 配置方式：`delivery\Configure-Hooks.ps1 -Client workbuddy`，
  把 8 个生命周期事件写进 `~/.workbuddy\settings.json`（只动带 `# AIStatusLight.Beta1`
  标记的条目，动别人配置之前自动备份）。
- 状态文件：`~/.ai-traffic-light\sessions\workbuddy-*.json`。
- **改完配置建议重启 WorkBuddy**；实测新会话不改配置也能触发的是旧 hook——所以配完后重启一次最稳。
- 验证：WorkBuddy 里随便发一条消息 → 绿灯「1 项运行」；跑完 → 红灯「1 项待处理」。

### 3. 扣子（Coze）⚠️ 必须用专用快捷方式启动

扣子正常启动是「普通模式」，状态灯**看不见它**。识别原理是仅本机回环的调试端口
`127.0.0.1:19222`（不开放局域网），所以：

- **启动扣子 → 用桌面「扣子（AI 状态灯）」图标**。它背后运行
  `releases\Start-Coze-Monitoring.ps1`：顺带拉起状态灯，检测到扣子在普通模式运行时弹窗问你是否重启。
- 已开调试的扣子再点一次快捷方式不会反复重启，直接复用。
- 想关掉识别：正常退出扣子，以后用普通「扣子」图标启动即可。
- 注意文件夹里没有「扣子（AI 状态灯）.exe」这种东西——桌面那个只是快捷方式，本体是上面的 .ps1 脚本。

### 4. ZCode ✅ 无需配置

只读 ZCode 本地数据库（`~/.zcode/cli/db/db.sqlite`），递归汇总多层子代理；
子代理状态不明时不判完成。启动 ZCode 跑任务即亮。

### 5. Marvis ✅ 无需配置（未运行时灯灭）

只读方式访问本机已安装 Marvis 的会话数据库，只显示最近 24 小时记录。
Marvis 没开时灯灭，属正常。此接口不是公开 API，Marvis 升级后需复测。

### 6. TRAE（TRAE SOLO CN）⚠️ 用专用快捷方式可识别实时生成

两个信号源：
- **实时绿灯**：识别 SOLO 渲染进程的流式会话收集器（生成期间即亮绿）。
  需要用桌面「TRAE（AI 状态灯）」快捷方式启动 TRAE（仅本机回环调试端口 `127.0.0.1:19226`）。
  普通模式启动的 TRAE 看不到实时生成。
- **任务完成红灯**：只读全局状态数据库
  （`%APPDATA%\TRAE SOLO CN\User\globalStorage\state.vscdb` 的 solo 模式状态表），
  Work/Code/Design 等模式各自**当前会话**结束后按状态亮红/等待/异常，24 小时后过期。
  该表只在任务生命周期节点更新，普通聊天不触发——这是数据库本身的行为。
- 加密的 `ModularData/ai-agent/database.db` 不在读取范围内；全程只取元数据不取正文。

### 7. 豆包（Doubao）⚠️ 必须用专用快捷方式启动

与扣子同一套方案：正常启动的豆包灯看不见；识别走仅本机回环调试端口 `127.0.0.1:19224`。

- **启动豆包 → 用桌面「豆包（AI 状态灯）」图标**（交付版由 Install 自动创建）。
  它会拉起状态灯；检测到豆包在普通模式运行时弹窗问你是否重启。
- 豆包正在生成回复 → 绿灯；一次回复结束 → 红灯；已连接但还没生成过 → 灰灯"暂无状态"。
- 只读生成任务的计数，不读取任何聊天内容。
- 想关掉识别：正常退出豆包，以后用普通「豆包」图标启动即可。

> 说明：面板上「安装 Hooks」按钮写入的是 CodeBuddy / Claude / Codex 三家的通用
> Hook（跨平台）。但当前版本的汇总端只读取 Codex 和 WorkBuddy 的 Hook 状态文件，
> 所以 Claude / CodeBuddy 的灯**当前不亮**，属于已知范围限制，不是故障。

---

## 四、日常使用

- 悬停灯壳 → 展开各客户端会话明细；× 收起，≡ 展开，Esc 关闭。
- 拖动灯壳移动位置。
- 托盘菜单：「显示悬浮灯」「开机自启动」开关、「退出」。
- 没有任何客户端在跑时，悬浮灯自动隐藏（托盘还在）。

## 五、灯不亮？按这个顺序排查

1. **客户端在运行吗？** 悬浮灯只在识别到客户端进程时显示（WorkBuddy.exe / Coze.exe 等）。
2. **扣子是不是普通模式启动的？** 用「扣子（AI 状态灯）」快捷方式重启它。
3. **WorkBuddy 配置生效了吗？** 看 `~\.workbuddy\settings.json` 的 hooks 里有没有
   `# AIStatusLight.Beta1` 标记的 EncodedCommand 条目；配置过就重启一次 WorkBuddy。
4. **状态文件有动静吗？** 跑一条任务后看 `~\.ai-traffic-light\sessions\` 里有没有
   `workbuddy-*.json` / `codex-*.json` 更新。
5. **端口冲突？** 扣子识别只认 127.0.0.1:19222；若被其他程序占用，启动脚本会明确报错拒绝。
6. 还不行 → 托盘退出状态灯，重新打开；再不行看第七节反馈问题时带上
   `~\.ai-traffic-light\sessions\` 的文件列表（只有状态元数据，没有聊天内容）。

## 六、卸载

- 交付版：安装目录里 **Uninstall.cmd**——只删除自己写入的 Hooks（按标记识别，不碰其他配置）、
  快捷方式和程序文件。
- 手动删 Hooks：`delivery\Configure-Hooks.ps1 -Client workbuddy -Uninstall`（Codex 同理 `-Client codex`）。

---

## 七、macOS 现状（诚实版）

**结论：部分可用，需要在 Mac 上自行构建，且 5 个灯里只有 Codex 有把握，其余未移植。**
以下基于源码审查，**未在真实 Mac 上验证**。

### 能用的

| 能力 | 状态 |
|---|---|
| 构建与运行（托盘 + 悬浮灯） | 代码是跨平台的（Tauri），需自行构建 |
| Codex 灯（本地数据库 `~/.codex/state_5.sqlite`） | 代码路径平台无关；**Mac 版 Codex 是否用同一数据库未验证** |
| Codex 灯（Hooks 方式） | `hooks/status_writer.py`（python3）+ 面板「安装 Hooks」按钮都支持 macOS |
| 悬浮窗显隐 | 2026-09-07 已修：进程名匹配此前只认 `.exe` 后缀，Mac 上会永远隐藏；现在两种形式都认 |

### 不能用的（Windows 专用，待移植）

- **WorkBuddy 灯**：Hooks 配置脚本和采集器都是 PowerShell，且本地适配器依赖
  `%LOCALAPPDATA%`。移植需要：bash/python 版 Configure 脚本 + `status_writer.py` 接入。
- **扣子灯**：CDP 采集脚本是 PowerShell。CDP 协议本身跨平台，移植成 python3 采集器即可，
  工作量不大但需要实机验证。
- **Marvis 灯**：采集器调用 `powershell.exe` 读 SQLite。需改成直接用 Rust 读库或 python3。
- **ZCode 灯**：数据库路径 `~/.zcode/cli/db/db.sqlite` 本身是家目录相对路径，代码平台无关，
  **Mac 上有较大机会直接可用，但未验证**。

### macOS 构建步骤（在 Mac 上执行）

```bash
# 依赖：Xcode Command Line Tools、Rust、Node 18+、pnpm
pnpm install --frozen-lockfile
pnpm build
pnpm tauri build        # 产出 .app / .dmg；调试可加 --no-bundle
```

首次运行后点面板「安装 Hooks」写入 Codex（以及 Claude/CodeBuddy）的 Hook 配置，
新建会话即亮灯。

### 已知限制

- 未签名、未公证：macOS 首次打开要在「系统设置 → 隐私与安全性」里手动放行。
- 灯的显隐依赖客户端进程名（Codex / WorkBuddy / Coze / ZCode / Marvis，大小写不敏感），
  客户端改名后需要同步更新 `presence.rs`。
- Mac 上没有「扣子专用启动快捷方式」，就算移植了 CDP 采集，也需要对应做一个启动器。

---

## 九、团队面板(内网,可选)

团队内网部署后,管理员可以在浏览器里看到每个成员:在线状态、在用哪些 AI 客户端、
各会话状态与项目名。**上报内容只有成员名、机器名、软件名、状态、项目名,
不含会话内容**;客户端默认关闭上报,未配置时零网络行为。

**管理员(一台内网常开机器):**

1. 构建:`cargo build --release -p ai-light-team-server`(workspace 根目录执行)。
2. 启动:双击 `scripts\Start-Team-Server.ps1`(默认端口 8787),或手动运行
   `target
eleasei-light-team-server.exe --port 8787`。
3. 首次启动会在 exe 同目录生成 `server.db`(数据)和 `admin-token.txt`(管理密钥)。
4. 浏览器打开控制台提示的地址,粘贴管理密钥,即可看到设备面板;改名/删除按钮在每台设备卡片上。

**成员(自己的机器):**

1. 点悬浮灯面板右上角「团队」按钮。
2. 勾选「启用团队上报」,填管理员给的服务器地址(如 `http://192.168.x.x:8787`),
   显示名可不填(默认用机器名),保存。设备标识自动生成并保存在本机。
3. 关闭上报:取消勾选保存即可,立刻停止。

**隐私边界**:数据存管理员机器的 `server.db`;想退出直接关开关或让管理员删设备。
报错会在面板状态行显示(如"服务器返回 401")。

---

## 附：关键位置速查

| 东西 | 位置 |
|---|---|
| 状态文件目录 | `~/.ai-traffic-light/sessions/`（JSON 元数据，24 小时自动过期） |
| Hook 写入器（Windows） | `~/.ai-traffic-light/hooks/status_writer.ps1` |
| WorkBuddy Hooks 配置 | `~/.workbuddy/settings.json` 的 `hooks` 字段 |
| 扣子调试端口 | `127.0.0.1:19222`（仅本机回环） |
| 豆包调试端口 | `127.0.0.1:19224`（仅本机回环） |
| 状态灯本体（本机） | `D:\3Dauto\ai-traffic-light\releases\AI-Status-Light-Recovery.exe` |
| 扣子启动脚本（本机） | `D:\3Dauto\ai-traffic-light\releases\Start-Coze-Monitoring.ps1` |
| 团队面板后端 | `target\release\ai-light-team-server.exe`（数据 `server.db`、密钥 `admin-token.txt` 在 exe 同目录） |
| 团队上报配置（成员机） | `~/.ai-traffic-light/config.json` 的 `team` 字段 |
| 排查记录 | 项目根目录 `RECOVERY-STATUS.md`、`DESKTOP-ADAPTER-INVESTIGATION.md` |
