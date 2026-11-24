import sys,json
for l in sys.stdin:
    l=l.strip()
    if not l.startswith('{'): print(l); continue
    d=json.loads(l)
    if d.get('cmd')=='ingest' and 'phase_seconds' in d:
        print(d.get('label'), 'T',d['threads'], 's=%.3f'%d['seconds'], 'GB/s=%.2f'%d['text_gb_per_s'], 'Medges/s=%.1f'%(d['edges_per_s']/1e6), 'rss=%.2f'%d['peak_rss_gb'], 'tmpGB=%.2f'%d['temp_disk_gb'], {k:round(v,3) for k,v in d['phase_seconds'].items() if v>0.001}, 'thr-s', {k:round(v,2) for k,v in d['thread_seconds'].items() if v>0.005}, 'load', d['loadavg_1m_at_end'])
    elif d.get('cmd')=='run':
        print(d.get('label',''), d['graph'], d['algo'], 'T',d['threads'], d['load'], d['order'], d['ids'], 'best=%.3f'%d['seconds_best'], [round(x,3) for x in d['seconds_all']], 'load_s=%.2f'%d['load_seconds'], 'rss=%.2f'%d['peak_rss_gb'], json.dumps(d['result'])[:230], 'load', d['loadavg_1m_at_end'])
    elif d.get('cmd')=='semiext':
        print(d.get('label',''), d['graph'], d['algo'], d['io'], 'T',d['threads'], 's=%.2f'%d['seconds'], 'iters', [round(x,2) for x in d['iter_seconds'][:6]], 'readGB=%.1f'%d['bytes_read_gb'], 'GB/s=%.2f'%d['read_gb_per_s_wall'], 'io_thr_s=%.1f'%d['io_thread_seconds'], 'rss=%.2f anon=%.2f'%(d['peak_rss_gb'],d['rss_anon_gb']), json.dumps(d['result'])[:200], 'load', d['loadavg_1m_at_end'])
    else: print({k:(round(v,3) if isinstance(v,float) else v) for k,v in d.items()})
