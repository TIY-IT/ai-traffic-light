// Read only the mounted Coze conversation's status metadata. No message bodies.
// Tier 1: React fiber walk for the exact sessionStatus fields (current builds).
// Tier 2 (universal): a one-time in-page network hook — a Coze reply always arrives as a
// streamed fetch/XHR with many chunks over seconds, regardless of internal object names.
(() => {
  const root = document.getElementById('root');
  if (!root || location.protocol !== 'coze-desktop:') return [];
  const out = [];
  const key = Object.keys(root).find(k => k.startsWith('__reactContainer$'));
  if (key) {
    const container = root[key];
    const pending = [container?.stateNode?.current || container], seen = new Set();
    const rows = new Map(), names = new Map();
    const currentConversation = new URL(location.href).searchParams.get('conversationId');
    while (pending.length && seen.size < 30000) {
      const fiber = pending.pop();
      if (!fiber || seen.has(fiber)) continue;
      seen.add(fiber); pending.push(fiber.child, fiber.sibling);
      const p = fiber.memoizedProps;
      if (!p || typeof p !== 'object') continue;
      if (p.sessionInfo?.id && typeof p.sessionInfo.name === 'string') names.set(String(p.sessionInfo.id), p.sessionInfo.name.slice(0,80));
      if (typeof p.sessionId !== 'string' || typeof p.sessionStatus !== 'number') continue;
      if (!/^\d{1,32}$/.test(p.sessionId)) continue;
      const conversation = p.conversationId || currentConversation;
      if (conversation && !/^\d{1,32}$/.test(conversation)) continue;
      const id = conversation || p.sessionId;
      const old = rows.get(id);
      // Conflicting observations fail closed instead of selecting a green status.
      rows.set(id, {id, sessionId:p.sessionId, status:old && old.status !== p.sessionStatus ? -1 : p.sessionStatus});
    }
    for (const r of rows.values()) out.push({...r, title:names.get(r.sessionId)||'扣子当前会话'});
  }
  if (out.length) return out;
  // Tier 2: universal streaming watch (reacts to any streamed reply on this build).
  const N = window.__AI_LIGHT_NET__ || (window.__AI_LIGHT_NET__ = { streams: [] });
  if (!N.hooked) {
    N.hooked = true;
    const done = (st) => { st.pending = false; st.lastEnd = Date.now(); };
    if (window.fetch && !String(window.fetch).includes('__aiLight')) {
      const orig = window.fetch;
      const wrapped = function() {
        const st = { pending: true, chunks: 0, first: 0, last: 0 };
        N.streams.push(st);
        if (N.streams.length > 50) N.streams.splice(0, N.streams.length - 50);
        let p;
        try { p = orig.apply(this, arguments); } catch (e) { done(st); throw e; }
        p.then((resp) => {
          try {
            if (resp.body && resp.clone) {
              const reader = resp.clone().body.getReader();
              const pump = () => reader.read().then((r) => {
                if (r.done) { done(st); return; }
                st.chunks++; if (!st.first) st.first = Date.now(); st.last = Date.now();
                pump();
              }, () => done(st));
              pump();
            } else { done(st); }
          } catch (e) { done(st); }
        }, () => done(st));
        return p;
      };
      wrapped.__aiLight = true;
      window.fetch = wrapped;
    }
    if (window.XMLHttpRequest && !window.XMLHttpRequest.prototype.__aiLight) {
      const openOrig = window.XMLHttpRequest.prototype.open;
      const sendOrig = window.XMLHttpRequest.prototype.send;
      window.XMLHttpRequest.prototype.open = function(m, u) {
        this.__st = { pending: true, chunks: 0, first: 0, last: 0 };
        N.streams.push(this.__st);
        if (N.streams.length > 50) N.streams.splice(0, N.streams.length - 50);
        return openOrig.apply(this, arguments);
      };
      window.XMLHttpRequest.prototype.send = function() {
        if (this.__st) {
          const st = this.__st;
          this.addEventListener('progress', () => { st.chunks++; if (!st.first) st.first = Date.now(); st.last = Date.now(); });
          this.addEventListener('loadend', () => done(st));
        }
        return sendOrig.apply(this, arguments);
      };
      window.XMLHttpRequest.prototype.__aiLight = true;
    }
  }
  let active = 0;
  const nowMs = Date.now();
  for (const st of N.streams) {
    if (st.pending && st.chunks >= 3 && nowMs - st.first > 1500) { active = 1; }
  }
  if (active) {
    out.push({ id: 'coze-live', live: true, status: 1, title: '扣子当前会话' });
  } else if (N.lastGenEnd && nowMs - N.lastGenEnd < 6 * 3600 * 1000) {
    out.push({ id: 'coze-live', live: true, status: 2, title: '扣子当前会话' });
  }
  if (active) { N.lastGenEnd = 0; N.sawGen = true; }
  return out;
})()
