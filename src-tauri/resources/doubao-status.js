// Read only reply counters / streaming flags from the Doubao chat page. No message bodies.
// Signals are MERGED (strongest wins), never short-circuited: the reply-counter service does
// not track plain chats on some builds, while the network hook sees every streamed reply.
// Tier A (universal): in-page network hook counts chunk rhythm of streamed responses.
// Tier B: data-streaming DOM flags on streaming bubbles.
// Tier C: reply counter service (exact on builds that track plain chats).
(() => {
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
  if (document.querySelectorAll('[data-streaming="true"]').length > 0) active = 1;
  let ended = 0;
  const svc = window.__messageTaskAssociationService__;
  if (svc && svc.getSnapshot) {
    const s = svc.getSnapshot();
    if ((Number(s.activeReplyCount) || 0) > 0) active = 1;
    ended = Number(s.endedReplyCount) || 0;
  }
  return { connected: true, active: active, ended: ended };
})()
