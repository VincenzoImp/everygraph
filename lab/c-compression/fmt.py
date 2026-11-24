import json,sys
for l in sys.stdin:
  try: d=json.loads(l)
  except: print(l.strip()); continue
  s=d['codec']+' '+d['prefix'].split('/')[-1]+' bpe=%.2f'%d['bits_per_edge']
  for k,f in [('scan_1t_edges_per_s','scan1=%.0fM'),('scan_par_edges_per_s','scanP=%.0fM'),('pr_edges_per_s','prE/s=%.0fM'),('cold_scan_edges_per_s','cold=%.0fM'),('after_cold_rescan_edges_per_s','rewarm=%.0fM')]:
    if k in d: s+=' '+f%(d[k]/1e6)
  if 'rand_ns_per_list' in d: s+=' rand=%.0fns/list %.1fns/e par=%.0fns/list'%(d['rand_ns_per_list'],d['rand_ns_per_edge'],d['rand_par_ns_per_list'])
  print(s)
