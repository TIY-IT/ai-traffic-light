import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
import { setupPanelControls } from "./panel-controls";
import { formatPromptTime, promptPieces, type PromptItem } from "./prompt-display";

setupPanelControls();

type Status =
  | "idle"
  | "working"
  | "waiting"
  | "completed"
  | "error";

interface StatusSnapshot {
  state: Status;
  label: string;
  message: string;
  session_count: number;
  updated_at: number;
  sessions: SessionSnapshot[];
}

interface SessionSnapshot {
  id: string;
  client: string;
  title: string;
  state: Status;
  label: string;
  message: string;
  updated_at: number;
  created_at: number;
}

interface PromptPage {
  prompts: string[];
  items?: PromptItem[];
  note: string;
}

interface PromptView {
  id: string;
  note: string;
  prompts: PromptItem[];
  error: string;
  showAll: boolean;
  open: Set<number>;
}

const PREVIEW_PROMPTS = 5;
const knownClients = ["codex", "workbuddy", "marvis", "zcode", "coze", "trae", "doubao"];
let selectedSessionId: string | null = null;
let promptView: PromptView | null = null;

const housings = Array.from(document.querySelectorAll<HTMLElement>(".housing"));
const statusLabel = document.querySelector<HTMLElement>("#status-label");
const sessionCount = document.querySelector<HTMLElement>("#session-count");
const sessionList = document.querySelector<HTMLUListElement>("#session-list");
const hooksWarning = document.querySelector<HTMLElement>("#hooks-warning");
const hooksWarningMessage = document.querySelector<HTMLElement>(
  "#hooks-warning-message",
);
const hooksStatus = document.querySelector<HTMLElement>("#hooks-status");
const setupButton = document.querySelector<HTMLButtonElement>("#setup-hooks");
const clearSessionsButton =
  document.querySelector<HTMLButtonElement>("#clear-sessions");
const freshChip = document.querySelector<HTMLElement>("#fresh-chip");
let hooksSuccessVisibleUntil = 0;

function clientLabel(client: string) {
  if (client === "zcode") return "ZCode";
  if (client === "coze") return "扣子";
  if (client === "trae") return "TRAE";
  if (client === "doubao") return "豆包";
  if (client === "marvis") return "Marvis";
  if (client === "workbuddy") return "WorkBuddy";
  if (client === "codex") return "Codex";
  if (client === "claude") return "Claude";
  return "CodeBuddy";
}

const stateBadgeText: Record<Status, string> = {
  working: "运行中",
  completed: "已停止",
  waiting: "等待",
  error: "异常",
  idle: "空闲",
};

// "刚结束"提示仍用 15 分钟。灯本身不随这个阈值熄灭。
const FRESH_STOP_SECS = 15 * 60;
const ENDED_STATES: Status[] = ["completed", "waiting", "error"];

// 前端自测的结束提醒:模块级保存上一帧状态(id → state),首轮 render 只播种。
const prevState = new Map<string, string>();
let prevStateSeeded = false;
const attnTimers = new Map<string, number>();

let audioCtx: AudioContext | null = null;

// WebView2 的自动播放策略可能把 AudioContext 挂起:监听用户点击并尝试恢复。
// 不能用 {once:true}:ctx 是懒创建的,首次点击时可能还不存在。
document.addEventListener("pointerdown", () => {
  if (audioCtx && audioCtx.state === "suspended") {
    void audioCtx.resume().catch(() => {});
  }
});

function ensureAudioCtx(): AudioContext {
  if (!audioCtx) audioCtx = new AudioContext();
  return audioCtx;
}

// 两声短音(880Hz→660Hz),总时长约 0.3s,gain 峰值 0.12 指数衰减;失败静默。
async function playDing() {
  try {
    if (!(await invoke<boolean>("notify_enabled"))) return;
    const ctx = ensureAudioCtx();
    if (ctx.state !== "running") return;
    const start = ctx.currentTime;
    const gain = ctx.createGain();
    gain.gain.setValueAtTime(0.0001, start);
    gain.gain.exponentialRampToValueAtTime(0.12, start + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.3);
    gain.connect(ctx.destination);
    const tones: Array<[number, number]> = [
      [0, 880],
      [0.14, 660],
    ];
    for (const [offset, freq] of tones) {
      const osc = ctx.createOscillator();
      osc.type = "sine";
      osc.frequency.value = freq;
      osc.connect(gain);
      osc.start(start + offset);
      osc.stop(start + offset + 0.14);
    }
    window.setTimeout(() => gain.disconnect(), 500);
  } catch {
    // 提示音失败不影响状态灯。
  }
}

// 该 client 红灯闪烁 6 秒;重复触发先清旧 timer 再重新计时。
function flashAttn(housing: HTMLElement) {
  const red = housing.querySelector<HTMLElement>(".light-red");
  if (!red) return;
  const client = housing.dataset.client || "";
  const previous = attnTimers.get(client);
  if (previous !== undefined) clearTimeout(previous);
  red.classList.add("attn");
  attnTimers.set(
    client,
    window.setTimeout(() => {
      red.classList.remove("attn");
      attnTimers.delete(client);
    }, 6000),
  );
}

// Click-through map: the backend only accepts mouse events inside these
// window-relative logical rects, so empty transparent areas stop blocking
// clicks to windows underneath.
const panelStack = document.querySelector<HTMLElement>(".panel-stack");
let lastHitRegions = "";

function publishHitRegions() {
  const rects: Array<[number, number, number, number]> = [];
  const pushRect = (el: HTMLElement) => {
    const rect = el.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      rects.push([rect.left, rect.top, rect.width, rect.height]);
    }
  };
  housings.forEach((housing) => {
    if (!housing.hidden) pushRect(housing);
  });
  if (panelStack && getComputedStyle(panelStack).display !== "none") {
    panelStack.querySelectorAll<HTMLElement>(".panel").forEach((panel) => {
      if (!panel.hidden) pushRect(panel);
    });
  }
  const key = JSON.stringify(rects);
  if (key === lastHitRegions) return;
  lastHitRegions = key;
  invoke("set_hit_regions", { rects }).catch(() => {});
}

function render(snapshot: StatusSnapshot) {
  snapshot.session_count = snapshot.sessions.length;

  // 灯只在使用中出现:运行中绿灯,15 分钟内刚停红灯。更早的已停和暂无状态不渲染。
  const nowSecs = Date.now() / 1000;
  const isEnded = (s: SessionSnapshot) => ENDED_STATES.includes(s.state);
  const isFresh = (s: SessionSnapshot) =>
    isEnded(s) && nowSecs - s.updated_at <= FRESH_STOP_SECS;
  const hasRunning = snapshot.sessions.some((s) => s.state === "working");
  snapshot.label = hasRunning
    ? "运行中"
    : snapshot.sessions.some(isFresh)
      ? "有会话刚结束"
      : snapshot.sessions.some((s) => s.state !== "idle")
        ? "已停止 · 查看结果"
        : "暂无会话状态";
  document.body.dataset.state = snapshot.state;

  // 对比上一帧:working → 结束态的会话触发提醒(首轮只播种,不提醒)。
  const justEndedClients = new Set<string>();
  if (prevStateSeeded) {
    for (const session of snapshot.sessions) {
      if (prevState.get(session.id) === "working" && isEnded(session)) {
        justEndedClients.add(session.client);
      }
    }
  }

  for (const housing of housings) {
    const sessions = snapshot.sessions.filter(s => s.client === housing.dataset.client);
    const running = sessions.filter((s) => s.state === "working").length;
    const freshStopped = sessions.filter(isFresh).length;
    const showLamp = running > 0 || freshStopped > 0;
    housing.hidden = !showLamp;
    if (!showLamp) continue;
    const colorState = running ? "completed" : "waiting";
    housing.querySelectorAll<HTMLElement>(".light").forEach(light => {
      light.classList.toggle("active", light.dataset.state === colorState);
    });
    const label = housing.querySelector<HTMLElement>(".client-status");
    if (label) {
      label.textContent = running
        ? `${running} 运行${freshStopped ? ` · ${freshStopped} 新停` : ""}`
        : `刚结束 ${freshStopped}`;
    }
    housing.setAttribute("aria-label", `${clientLabel(housing.dataset.client || "")}：${label?.textContent}`);
    if (justEndedClients.has(housing.dataset.client || "")) flashAttn(housing);
  }

  const freshTotal = snapshot.sessions.filter(isFresh).length;
  if (freshChip) {
    freshChip.textContent = `刚结束 ${freshTotal}`;
    freshChip.hidden = freshTotal === 0;
  }

  if (statusLabel) statusLabel.textContent = snapshot.label;
  if (sessionCount) {
    const runningTotal = snapshot.sessions.filter(
      (session) => session.state === "working",
    ).length;
    const stoppedTotal = snapshot.sessions.filter((session) =>
      ["completed", "waiting", "error"].includes(session.state),
    ).length;
    sessionCount.textContent =
      snapshot.session_count > 0
        ? `${runningTotal} 运行 · ${stoppedTotal} 已停`
        : "暂无会话记录";
  }
  if (sessionList) {
    const extras = Array.from(
      new Set(
        snapshot.sessions
          .map((session) => session.client)
          .filter((client) => !knownClients.includes(client)),
      ),
    );
    const clients = [...knownClients, ...extras];
    const scrollTop = sessionList.scrollTop;
    sessionList.replaceChildren(
      ...clients.flatMap((client) => {
        const groupTitle = document.createElement("li");
        groupTitle.className = "session-group-title";
        groupTitle.textContent = clientLabel(client);

        const sessions = snapshot.sessions
          .filter((session) => session.client === client)
          // 组内排序:运行中的排最前,其余按 updated_at 倒序(最新在前)。
          .sort(
            (left, right) =>
              Number(right.state === "working") - Number(left.state === "working") ||
              right.updated_at - left.updated_at,
          )
          .map((session) => {
            const item = document.createElement("li");
            item.className = `session-item session-${session.state}`;
            if (selectedSessionId === session.id) item.classList.add("is-selected");

            const row = document.createElement("div");
            row.className = "session-row";

            const dot = document.createElement("span");
            dot.className = "session-dot";

            const badge = document.createElement("span");
            badge.className = "session-state-badge";
            badge.textContent = session.label || stateBadgeText[session.state];

            const remove = document.createElement("button");
            remove.className = "session-remove";
            remove.type = "button";
            remove.title = "删除会话记录";
            remove.setAttribute("aria-label", `删除 ${session.title}`);
            remove.textContent = "×";
            remove.hidden = true;
            remove.addEventListener("click", async (event) => {
              event.stopPropagation();
              remove.disabled = true;
              await invoke("remove_session", { id: session.id });
              await refresh();
            });

            const heading = document.createElement("span");
            heading.className = "session-heading";
            const title = document.createElement("strong");
            title.textContent = session.title;
            title.title = session.title;
            heading.append(title);

            row.append(dot, heading, badge, remove);
            item.append(row);
            item.title = session.title || "查看你的提问";
            item.addEventListener("click", (event) => {
              if ((event.target as HTMLElement | null)?.closest(".prompt-list")) return;
              void selectSession(session);
            });

            if (selectedSessionId === session.id && promptView?.id === session.id) {
              item.append(renderPromptList(promptView));
            }
            return item;
          });

        if (sessions.length === 0) return [];
        return [groupTitle, ...sessions];
      }),
    );
    sessionList.scrollTop = scrollTop;
  }
  if (clearSessionsButton) {
    // 快照里存在已结束会话时才显示"清空已停"入口。
    clearSessionsButton.hidden = !snapshot.sessions.some(isEnded);
  }
  // 每轮 render 末尾整体更新上一帧状态表,作为下一轮的比对基准。
  prevState.clear();
  for (const session of snapshot.sessions) {
    prevState.set(session.id, session.state);
  }
  prevStateSeeded = true;
  if (justEndedClients.size > 0) void playDing();
  fitWindowToWidget();
  publishHitRegions();
  syncLampPresence(housings.some((housing) => !housing.hidden));
}

let lampPresence: boolean | null = null;

function syncLampPresence(active: boolean) {
  if (lampPresence === active) return;
  lampPresence = active;
  invoke("set_lamp_presence", { active }).catch(() => {
    if (lampPresence === active) lampPresence = null;
  });
}

function fitWindowToWidget() {
  const widget = document.querySelector<HTMLElement>(".widget");
  if (!widget) return;
  const rect = widget.getBoundingClientRect();
  const width = Math.ceil(rect.width) + 12;
  const height = Math.ceil(rect.height) + 12;
  if (width >= 48 && height >= 48) {
    invoke("resize_light_to_fit", { width, height }).catch(() => {});
  }
}

function renderPromptList(view: PromptView): HTMLElement {
  const list = document.createElement("div");
  list.className = "prompt-list";
  const noteText = view.error || view.note;
  if (noteText) {
    const note = document.createElement("p");
    note.className = view.error ? "prompt-error" : "prompt-note";
    note.textContent = noteText;
    list.append(note);
  }
  const newestFirst = view.prompts.map((item, index) => ({ item, index })).reverse();
  const visible = view.showAll ? newestFirst : newestFirst.slice(0, PREVIEW_PROMPTS);
  for (const { item, index } of visible) {
    const card = document.createElement("article");
    card.className = "prompt-card";
    if (view.open.has(index)) card.classList.add("is-open");
    const time = formatPromptTime(item.at);
    if (time) {
      const stamp = document.createElement("div");
      stamp.className = "prompt-time";
      stamp.textContent = time;
      card.append(stamp);
    }
    const body = document.createElement("div");
    body.className = "prompt-body";
    for (const piece of promptPieces(item.text)) {
      if (piece.kind === "link" && piece.url) {
        const link = document.createElement("a");
        link.className = "prompt-link";
        link.href = piece.url;
        link.title = piece.title || piece.url;
        link.textContent = piece.text;
        link.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          void invoke("open_external_url", { url: piece.url });
        });
        body.append(link);
      } else {
        body.append(document.createTextNode(piece.text));
      }
    }
    card.append(body);
    card.addEventListener("click", (event) => {
      event.stopPropagation();
      if ((event.target as HTMLElement | null)?.closest("a")) return;
      if (!promptView || promptView.id !== view.id) return;
      if (promptView.open.has(index)) promptView.open.delete(index);
      else promptView.open.add(index);
      void refresh();
    });
    list.append(card);
  }
  if (view.prompts.length > PREVIEW_PROMPTS) {
    const more = document.createElement("button");
    more.type = "button";
    more.className = "prompt-more";
    more.textContent = view.showAll ? "收起" : "展开全部";
    more.addEventListener("click", (event) => {
      event.stopPropagation();
      if (!promptView || promptView.id !== view.id) return;
      promptView.showAll = !promptView.showAll;
      void refresh();
    });
    list.append(more);
  }
  return list;
}

async function selectSession(session: SessionSnapshot) {
  if (selectedSessionId === session.id) {
    selectedSessionId = null;
    promptView = null;
    await refresh();
    return;
  }
  selectedSessionId = session.id;
  promptView = {
    id: session.id,
    note: "正在读取你的提问…",
    prompts: [],
    error: "",
    showAll: false,
    open: new Set(),
  };
  await refresh();
  try {
    const page = await invoke<PromptPage>("session_user_prompts", {
      client: session.client,
      id: session.id,
    });
    if (selectedSessionId !== session.id) return;
    const items =
      page.items && page.items.length > 0
        ? page.items
        : page.prompts.map((text) => ({ text, at: 0 }));
    promptView = {
      id: session.id,
      note: page.note,
      prompts: items,
      error: "",
      showAll: false,
      open: new Set(),
    };
  } catch (error) {
    if (selectedSessionId !== session.id) return;
    promptView = {
      id: session.id,
      note: "",
      prompts: [],
      error: `读取失败：${String(error)}`,
      showAll: false,
      open: new Set(),
    };
  }
  await refresh();
}

async function refresh() {
  try {
    const snapshot = await invoke<StatusSnapshot>("get_status");
    render(snapshot);
  } catch (error) {
    console.error("Failed to refresh AI client status", error);
  }
}

async function refreshHooksStatus() {
  if (hooksWarning) hooksWarning.hidden = true;
}

function setHooksStatus(message: string, state: "busy" | "error" | "success") {
  if (!hooksStatus) return;
  hooksStatus.hidden = false;
  hooksStatus.dataset.state = state;
  hooksStatus.textContent = message;
}

setupButton?.addEventListener("click", async () => {
  setupButton.disabled = true;
  setupButton.textContent = "正在安装…";
  setHooksStatus("正在写入 Hooks 配置，请稍候。", "busy");
  try {
    await invoke<string>("install_hooks");
    hooksSuccessVisibleUntil = Date.now() + 15000;
    setHooksStatus(
      "Hooks 已安装。请重启 AI 客户端并新建会话。Codex 桌面版请按新对话顶部提示信任 Hooks；CLI 可输入 /hooks 检查并信任。",
      "success",
    );
    await refreshHooksStatus();
  } catch (error) {
    setHooksStatus(`安装失败：${String(error)}`, "error");
  } finally {
    setupButton.textContent = "安装 Hooks";
    setupButton.disabled = false;
  }
});

clearSessionsButton?.addEventListener("click", async () => {
  clearSessionsButton.disabled = true;
  try {
    await invoke("clear_session_history");
    await refresh();
  } finally {
    clearSessionsButton.disabled = false;
  }
});

interface TeamConfig {
  enabled: boolean;
  server_url: string;
  display_name: string;
  device_token: string;
  interval_secs: number;
}

interface ReportStatus {
  last_ok: number;
  last_attempt: number;
  last_error: string | null;
}

const teamPanel = document.querySelector<HTMLElement>("#team-panel");
const teamEnabled = document.querySelector<HTMLInputElement>("#team-enabled");
const teamUrl = document.querySelector<HTMLInputElement>("#team-url");
const teamName = document.querySelector<HTMLInputElement>("#team-name");
const teamSave = document.querySelector<HTMLButtonElement>("#team-save");
const teamStatus = document.querySelector<HTMLElement>("#team-status");
const openTeamButton = document.querySelector<HTMLButtonElement>("#open-team");
const closeTeamButton = document.querySelector<HTMLButtonElement>("#close-team");
let latestTeamConfig: TeamConfig | null = null;

function setTeamStatus(text: string, state?: "ok" | "error") {
  if (!teamStatus) return;
  teamStatus.textContent = text;
  if (state) teamStatus.dataset.state = state;
  else delete teamStatus.dataset.state;
}

function sinceText(secs: number) {
  const elapsed = Math.max(0, Math.floor(Date.now() / 1000) - secs);
  return elapsed < 60 ? `${elapsed} 秒前` : `${Math.floor(elapsed / 60)} 分钟前`;
}

async function refreshTeamConfig() {
  if (!teamPanel || teamPanel.hidden) return;
  try {
    const [config, status] = await Promise.all([
      invoke<TeamConfig>("get_team_config"),
      invoke<ReportStatus>("report_status"),
    ]);
    latestTeamConfig = config;
    if (teamUrl && document.activeElement !== teamUrl) teamUrl.value = config.server_url;
    if (teamName && document.activeElement !== teamName) teamName.value = config.display_name;
    if (teamEnabled) teamEnabled.checked = config.enabled;
    if (!config.enabled) setTeamStatus("未启用");
    else if (status.last_error) setTeamStatus(`上报失败:${status.last_error}`, "error");
    else if (status.last_ok) setTeamStatus(`已上报 · ${sinceText(status.last_ok)}`, "ok");
    else setTeamStatus("等待首次上报…");
  } catch (error) {
    console.error("Failed to load team config", error);
  }
}

openTeamButton?.addEventListener("click", () => {
  if (!teamPanel) return;
  teamPanel.hidden = false;
  publishHitRegions();
  refreshTeamConfig();
});

closeTeamButton?.addEventListener("click", () => {
  if (teamPanel) teamPanel.hidden = true;
  publishHitRegions();
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && teamPanel && !teamPanel.hidden) {
    teamPanel.hidden = true;
    publishHitRegions();
  }
});

teamSave?.addEventListener("click", async () => {
  if (!teamSave || !teamEnabled) return;
  teamSave.disabled = true;
  setTeamStatus("保存中…");
  try {
    const merged: TeamConfig = {
      enabled: teamEnabled.checked,
      server_url: teamUrl?.value ?? "",
      display_name: teamName?.value ?? "",
      device_token: latestTeamConfig?.device_token ?? "",
      interval_secs: latestTeamConfig?.interval_secs ?? 30,
    };
    await invoke<TeamConfig>("set_team_config", { config: merged });
    setTeamStatus(teamEnabled.checked ? "已保存,正在上报…" : "已保存,上报关闭", "ok");
  } catch (error) {
    setTeamStatus(`保存失败:${String(error)}`, "error");
  } finally {
    teamSave.disabled = false;
  }
});

refresh();
refreshHooksStatus();
publishHitRegions();
const widgetRoot = document.querySelector<HTMLElement>(".widget");
if (widgetRoot) {
  new ResizeObserver(() => {
    fitWindowToWidget();
    publishHitRegions();
  }).observe(widgetRoot);
}
// The shell shows the window without waiting for this. A custom-protocol
// miss must not leave the HWND hidden.
invoke("window_ready").catch(() => {});
document.addEventListener("hit-regions-dirty", () => publishHitRegions());
window.setInterval(refresh, 200);
window.setInterval(refreshHooksStatus, 5000);
window.setInterval(refreshTeamConfig, 5000);
