// Live streaming signal for the TRAE SOLO workbench, read-only.
// The session collector ignores plain code chats, so two DOM fallbacks matter:
// while a turn streams, a stop button and a --streaming container appear.
// The network hook (installed once) adds a third signal that survives UI rewrites:
// a chat reply always arrives as a streamed fetch/XHR with many chunks over seconds.
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
  const c = window.__CHAT_SESSION_COLLECTOR__;
  let streams = (c && c.sessions && typeof c.sessions.size === 'number') ? c.sessions.size : 0;
  if (document.querySelector('[class*="--streaming"]')) streams += 1;
  if (document.querySelectorAll('[class*="stop"]').length > 0) streams += 1;
  for (const st of N.streams) {
    if (st.pending && st.chunks >= 3 && Date.now() - st.first > 1500) { streams += 1; break; }
  }
  return { connected: true, streams: streams };
})()
