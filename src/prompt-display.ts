export interface PromptItem {
  text: string;
  at: number;
}

const URL_LIMIT = 42;

export function shortenUrl(raw: string): string {
  const trimmed = raw.trim();
  try {
    const url = new URL(trimmed);
    const host = url.hostname.replace(/^www\./i, "");
    const parts = url.pathname.split("/").filter(Boolean).map(decodePath);
    const path = parts.length ? `/${parts.join("/")}` : "";
    const shown = `${host}${path}${url.search}${url.hash}`;
    if (shown.length <= URL_LIMIT) return shown;
    const last = parts[parts.length - 1] ?? "";
    const lastShort = last.length > 16 ? `…${last.slice(-14)}` : last;
    const first = parts[0] ?? "";
    const head = first && parts.length > 1 ? `${host}/${first}` : host;
    const body = lastShort ? `${head}/…/${lastShort}` : `${head}/…`;
    return body.length <= URL_LIMIT ? body : `${body.slice(0, URL_LIMIT - 1)}…`;
  } catch {
    return trimmed.length <= URL_LIMIT ? trimmed : `${trimmed.slice(0, URL_LIMIT - 1)}…`;
  }
}

function decodePath(part: string): string {
  try {
    return decodeURIComponent(part);
  } catch {
    return part;
  }
}

/** 刚刚 / N分钟前 / 北京时间 HH:mm。跨日时前面加上 M月D日。 */
export function formatPromptTime(at: number, nowMs = Date.now()): string {
  if (!at) return "";
  const delta = Math.round(nowMs / 1000) - at;
  if (delta < 60) return "刚刚";
  if (delta < 3600) return `${Math.max(1, Math.floor(delta / 60))}分钟前`;
  const when = new Date(at * 1000);
  const time = new Intl.DateTimeFormat("en-GB", {
    timeZone: "Asia/Shanghai",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(when);
  const dayKey = shanghaiDay(when);
  if (dayKey === shanghaiDay(new Date(nowMs))) return time;
  const [, month, day] = dayKey.split("-");
  return `${Number(month)}月${Number(day)}日 ${time}`;
}

function shanghaiDay(date: Date): string {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).format(date);
}

export interface PromptPiece {
  kind: "text" | "link";
  text: string;
  url?: string;
  title?: string;
}

/** Split prompt text into plain runs and shortened clickable links. */
export function promptPieces(text: string): PromptPiece[] {
  const pieces: PromptPiece[] = [];
  let index = 0;
  while (index < text.length) {
    const link = markdownLinkAt(text, index) ?? rawUrlAt(text, index);
    if (link) {
      const labelIsUrl =
        link.label === link.url ||
        link.label.replace(/\/$/, "") === link.url.replace(/\/$/, "") ||
        link.label.startsWith("http://") ||
        link.label.startsWith("https://");
      pieces.push({
        kind: "link",
        text: labelIsUrl ? shortenUrl(link.url) : link.label,
        url: link.url,
        title: link.url,
      });
      if (link.suffix) pieces.push({ kind: "text", text: link.suffix });
      index = link.next;
      continue;
    }
    const next = nextSpecial(text, index + 1);
    pieces.push({ kind: "text", text: text.slice(index, next) });
    index = next;
  }
  return pieces;
}

function nextSpecial(text: string, from: number): number {
  let at = text.length;
  for (const token of ["[", "http://", "https://"]) {
    const found = text.indexOf(token, from);
    if (found >= 0 && found < at) at = found;
  }
  return at;
}

interface FoundLink {
  url: string;
  label: string;
  suffix: string;
  next: number;
}

function markdownLinkAt(text: string, index: number): FoundLink | null {
  if (text[index] !== "[") return null;
  const close = text.indexOf("](", index + 1);
  if (close < 0 || close > index + 400) return null;
  const end = text.indexOf(")", close + 2);
  if (end < 0 || end > close + 2000) return null;
  const label = text.slice(index + 1, close).trim();
  const url = text.slice(close + 2, end).trim();
  if (!url.startsWith("http://") && !url.startsWith("https://")) return null;
  return { url, label, suffix: "", next: end + 1 };
}

function rawUrlAt(text: string, index: number): FoundLink | null {
  const rest = text.slice(index);
  if (!rest.startsWith("http://") && !rest.startsWith("https://")) return null;
  const match = /^https?:\/\/[^\s<>\]]+/.exec(rest);
  if (!match) return null;
  let url = match[0];
  let suffix = "";
  while (/[),.;:!?，。；、）>]$/.test(url)) {
    suffix = url.slice(-1) + suffix;
    url = url.slice(0, -1);
  }
  if (!url.startsWith("http")) return null;
  return { url, label: url, suffix, next: index + match[0].length };
}
