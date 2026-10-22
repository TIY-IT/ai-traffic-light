"""Read only Marvis conversation metadata; never read message contents."""
import datetime as dt
import json
import os
from pathlib import Path
import sqlite3
import time

def epoch(value):
    try:
        return dt.datetime.fromisoformat(value).timestamp()
    except (ValueError, TypeError):
        return 0

def classify(status, event, age, pending):
    if status in ('completed', 'cancelled', 'failed', 'error'):
        return ('error', '异常') if status in ('failed', 'error') else ('completed', '已结束')
    if event == 'RUN_ERROR':
        return 'error', '异常'
    if event == 'RUN_FINISHED':
        return 'completed', '已结束'
    if age > 120:
        return 'idle', '状态过期 / 待确认'
    if pending:
        return 'waiting', '等待确认'
    if status == 'in_progress':
        return 'working', '运行中'
    return 'idle', '状态未知'

def read():
    root = Path(os.environ['APPDATA']) / 'Tencent/Marvis/User'
    files = list(root.glob('*/database/data.db'))
    if not files:
        return []
    def modified(p):
        wal = Path(str(p) + '-wal')
        return max(p.stat().st_mtime, wal.stat().st_mtime if wal.exists() else 0)
    file = max(files, key=modified)
    now = time.time()
    result = []
    with sqlite3.connect(file.as_uri() + '?mode=ro', uri=True, timeout=0.2) as db:
        db.execute('PRAGMA query_only=ON')
        rows = db.execute('SELECT conversation_id,title,status,updated_at FROM conversations ORDER BY updated_at DESC LIMIT 30').fetchall()
        for cid, title, status, updated in rows:
            recent = db.execute('SELECT event_type,timestamp FROM agui_events WHERE conversation_id=? ORDER BY rowid DESC LIMIT 1', (cid,)).fetchone()
            stamp = max(epoch(updated), epoch(recent[1]) if recent else 0)
            if now - stamp > 86400:
                continue
            pending = db.execute("SELECT 1 FROM approvals WHERE conversation_id=? AND status='pending' LIMIT 1", (cid,)).fetchone()
            state, label = classify(status, recent[0] if recent else '', now - stamp, bool(pending))
            short_title = ' '.join((title or ('Marvis '+cid[-8:])).lstrip('\ufeff').split())[:80]
            result.append(dict(id='marvis:'+cid, client='marvis', title=short_title, state=state, label=label, message=label, updated_at=int(stamp), created_at=0))
    return result

if __name__ == '__main__':
    print(json.dumps(read(), ensure_ascii=True))
