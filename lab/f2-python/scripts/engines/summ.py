import json,sys
for f in sys.argv[1:]:
    r=json.load(open(f)); out={}
    for k,v in r.items():
        if isinstance(v,dict) and 'n' in v and 'timeouts' in v: out[k]=(round(v.get('median_ms',-1),3), round(v.get('p90_ms',-1),3), f"to={v['timeouts']} err={v['errors']} n={v['n']}")
        elif k!='check': out[k]=v
    print(f, json.dumps(out,default=str,indent=0)); print('check', r.get('check'))
