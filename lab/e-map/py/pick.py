# pick a deterministic zoom target (a mid-sized finest community inside the largest top-level community)
import sys, json, numpy as np
lay, graph = sys.argv[1], sys.argv[2]
dt = np.dtype([('parent','<u4'),('start','<u4'),('end','<u4'),('comm','<u4'),('level','i1'),('depth','u1'),('flags','u1'),('pad','u1'),
               ('x','<f4'),('y','<f4'),('r','<f4'),('node_r','<f4'),('nkids','<u4'),('internal','<u8'),('volume','<u8')])
tn = np.fromfile(lay + '/tnodes.bin', dtype=dt)
lp = np.fromfile(lay + '/leafparent.u32', dtype='<u4')
top = np.fromfile(lay + '/top.u32', dtype='<u4')
order = np.fromfile(lay + '/order.u32', dtype='<u4')
alive = (tn['flags'] & 4) == 0
leaf = alive & ((tn['flags'] & 2) != 0)
size = (tn['end'] - tn['start'])
nr = tn['node_r'][leaf]
print('tnodes', alive.sum(), 'leafparents', leaf.sum(), 'node_r pct', np.percentile(nr, [1, 10, 50, 90, 99]), file=sys.stderr)
print('leafparent r pct', np.percentile(tn['r'][leaf], [1, 10, 50, 90, 99]), 'depth hist', np.bincount(tn['depth'][alive]), file=sys.stderr)
# candidates: leaf parents of a readable size, preferring the largest top-level community;
# progressively looser filters so the script never comes up empty on fine-grained hierarchies
c = None
for lo, hi, want_top0 in [(40, 80, True), (40, 80, False), (20, 120, True), (20, 120, False), (8, 400, False), (2, 10**9, False)]:
    cand = np.where(leaf & (size >= lo) & (size <= hi))[0]
    if want_top0 and len(cand):
        cand = cand[top[order[tn['start'][cand]]] == 0]
    if len(cand) >= 10:
        dens = tn['internal'][cand] / size[cand]
        c = cand[np.argsort(-dens, kind='stable')[len(cand) // 10]]
        print('filter used: size', lo, hi, 'top0' if want_top0 else 'any top', 'candidates', len(cand), file=sys.stderr)
        break
assert c is not None, 'no leaf parent found'
ids = np.fromfile(graph + '/ids.u64', dtype='<u8')
node = int(order[tn['start'][c]])
json.dump({'tnode': int(c), 'x': float(tn['x'][c]), 'y': float(tn['y'][c]), 'r': float(tn['r'][c]), 'size': int(size[c]),
           'internal': int(tn['internal'][c]), 'node': node, 'orig': int(ids[node]), 'virtual': bool(tn['flags'][c] & 1)}, sys.stdout)
