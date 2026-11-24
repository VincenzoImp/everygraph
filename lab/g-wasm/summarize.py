# python3 -I summarize.py  -> markdown tables from results/*.jsonl
import json, os, sys
R = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'results')
def load(f):
    p = os.path.join(R, f)
    return [json.loads(l) for l in open(p) if l.strip().startswith('{')] if os.path.exists(p) else []
def f(x, k):
    v = x.get(k); return '' if v is None else (f'{v:.3f}' if isinstance(v, float) else str(v))
def ds(x):
    if x.get('mode') == 'text2': return 'lj-text'
    if 'text_bytes' in x and x.get('mode') in ('text', 'text2') and not (x.get('input') or x.get('ds')):
        return 'lj-text' if x['text_bytes'] > 1e8 else 'lj10-text'
    i = x.get('input') or x.get('ds') or x.get('path') or ''
    return 'lj10-text' if 'lj10' in i else ('lj-text' if 'lj.txt' in i else ('lj-csr' if x.get('mode') == 'csr' else i))
cols = ['parse_s', 'csr_s', 'ingest_two_pass_s', 'pr_s', 'cc_s', 'wasm_mem_bytes']
print('| runtime | variant | flags | threads | dataset | mode | ' + ' | '.join(cols) + ' | ok |')
print('|' + '---|' * (7 + len(cols)))
for src in ['native.jsonl', 'node.jsonl', 'chromium.jsonl']:
    for x in load(src):
        rt = 'native' if src == 'native.jsonl' else x.get('runtime', '')
        fl = x.get('node_flags', x.get('chrome_flags', ''))
        print(f"| {rt} | {x.get('variant','rayon')} | {fl} | {x.get('threads')} | {ds(x)} | {x.get('mode')} | " + ' | '.join(f(x, c) for c in cols) + f" | {x.get('ok', True)} |")
print()
print('| ceiling run | variant | m (undirected) | n | entries | wasm mem after CSR (GB) | build s | ok | error |')
print('|---|---|---|---|---|---|---|---|---|')
for x in load('ceiling.jsonl'):
    mem = x.get('mem_after_csr') or x.get('wasm_mem_bytes') or 0
    print(f"| {x.get('mode')} | {x.get('variant')} | {x.get('m','')} | {x.get('n','')} | {x.get('entries','')} | {mem/1e9:.2f} | {f(x,'csr_s') or f(x,'gen_csr_s')} | {x.get('ok')} | {(x.get('error') or '')[:60]} |")
