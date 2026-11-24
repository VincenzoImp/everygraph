# Render evaluation images (Rust rasterizer -> PPM), burn captions in, build the contact sheet.
import sys, json, subprocess, os, numpy as np
from PIL import Image, ImageDraw, ImageFont
E = '/home/tux/graphlab/exp/e-map'
graph = sys.argv[1]
G = f'/home/tux/graphlab/data/csr/{graph}'
gmap = f'{E}/gmap/target/release/gmap'
tgt = json.load(open(f'{E}/out/{graph}-hcl/target.json'))
meta = json.load(open(G + '/meta.json'))
def font(sz):
    for p in ['/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf', '/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf']:
        if os.path.exists(p): return ImageFont.truetype(p, sz)
    return ImageFont.load_default()
methods = [('hcl', 'hierarchy-constrained circle layout (ours)'), ('fastrp', 'FastRP-like embedding + PCA to 2D (baseline)')]
views = [('0-whole', 0.5, 4096, 0, 'whole graph'), ('1-zoom', 0.08, 1600, 0, 'zoom 6x: one continent'),
         ('2-zoom', 0.012, 1600, 0, 'zoom 42x: regions and finest communities'),
         ('3-zoom', 0.002, 1600, 400000, 'zoom 250x: nodes + all edges touching the view'),
         ('4-zoom', 0.0006, 1600, 400000, 'zoom 830x: individual nodes and edges')]
extra = [('hcl', '1-zoom-rawedges', 0.08, 1600, 400000, 'zoom 6x with RAW edges (sampled): the hairball that aggregation must avoid')]
thumbs = {}
def render(m, mname, tag, half, size, edges, cap):
    lay = f'{E}/out/{graph}-{m}'
    if not os.path.exists(lay + '/pos.f32'): return None
    if tag.startswith('0'):
        cx = cy = 0.5
    elif m == 'hcl':
        cx, cy = tgt['x'], tgt['y']
    else:
        pos = np.memmap(lay + '/pos.f32', dtype='<f4', mode='r').reshape(-1, 2); cx, cy = map(float, pos[tgt['node']])
    ppm = f'{E}/tmp/r.ppm'
    cmd = [gmap, 'render', '--layout', lay, '--graph', G, '--out', ppm, '--size', str(size), '--cx', str(cx), '--cy', str(cy), '--half', str(half)]
    if edges: cmd += ['--edges', str(edges)]
    if m != 'hcl': cmd += ['--node-r', '0.000021']
    info = json.loads(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout.strip().splitlines()[-1])
    im = Image.open(ppm).convert('RGB'); d = ImageDraw.Draw(im); fs = max(18, size // 70)
    line1 = f'{graph} ({meta["n"]:,} nodes, {meta["m_undirected"]:,} edges) - {mname}'
    line2 = f'{cap} | view width {2*half:g} of the map | {info["nodes_in_view"]:,} nodes in view' + (f' | {info["edges_drawn"]:,} of {info["edges_in_view"]:,} edges drawn' if edges else ' | colour = top-level community; outlines = communities (faint = derived levels)')
    d.rectangle([0, 0, size, fs * 3], fill=(0, 0, 0)); d.text((fs // 2, fs // 4), line1, fill=(255, 255, 255), font=font(fs)); d.text((fs // 2, fs * 1.5), line2, fill=(190, 200, 220), font=font(int(fs * 0.8)))
    out = f'{E}/img/{graph}-{m}-{tag}.png'; im.save(out, optimize=False); print(out, info, flush=True)
    return im
for m, mname in methods:
    for tag, half, size, edges, cap in views:
        im = render(m, mname, tag, half, size, edges, cap)
        if im is not None: thumbs[(m, tag)] = im.resize((800, 800), Image.LANCZOS)
for m, tag, half, size, edges, cap in extra:
    render(m, dict(methods)[m], tag, half, size, edges, cap)
rows = [m for m, _ in methods if (m, '0-whole') in thumbs]
sheet = Image.new('RGB', (800 * len(views), 800 * len(rows)), (0, 0, 0))
for r, m in enumerate(rows):
    for c, v in enumerate(views):
        sheet.paste(thumbs[(m, v[0])], (c * 800, r * 800))
sheet.save(f'{E}/img/contact-{graph}.png')
if graph == 'lj': sheet.save(f'{E}/img/contact.png')
