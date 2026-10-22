import runpy
reader = runpy.run_path('src-tauri/resources/marvis-status.py')
c = reader['classify']
assert c('in_progress', 'TOOL_CALL_ARGS', 1, False)[0] == 'working'
assert c('completed', 'RUN_FINISHED', 1, False)[0] == 'completed'
assert c('in_progress', 'RUN_ERROR', 1, False)[0] == 'error'
assert c('cancelled', '', 1, False)[0] == 'completed'
assert c('in_progress', '', 1, True)[0] == 'waiting'
assert c('in_progress', '', 300, False)[0] == 'idle'
assert c('created', '', 1, False)[0] == 'idle'
rows = reader['read']()
assert all(r['client'] == 'marvis' and len(r['title']) <= 80 for r in rows)
print('PASS: 7 state cases; live metadata states:', [r['state'] for r in rows])
