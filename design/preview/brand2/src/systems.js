// ===== Everygraph identity, round 2 — page script (inlined by build.py) =====
// TOKENS (src/tokens.json) and PAL (src/palettes.json, written by tools/palette.mjs) are defined above.
'use strict';
const IDS = Object.keys(TOKENS);
const OPT = (() => { const o = {}; location.hash.replace(/^#/, '').split('&').forEach(kv => { const [k, v] = kv.split('='); if (k) o[k] = decodeURIComponent(v || ''); }); return o; })();
window.addEventListener('hashchange', () => location.reload());

const SYS = {
  1: { primary: 'light', fav: 'seal',
       why: 'Direction B exactly as chosen in round 1 — black, white, one signal orange, square corners, uppercase monospace labels — with A\'s nested contours redrawn in B\'s weight. Everything else on this page is measured against it.',
       mistake: 'Keeping it unchanged. Black plus one orange is the most common look in developer tools; the orange sits ΔE 13.8 from the error red and ΔE 3 from the warning amber for a protanope; on the dark page a red light enough to read as text ends up ΔE 5.9 from the orange; and uppercase monospace is tiring past a few words.',
       mark: 'Heavy rings, orange dot. The square "seal" carries B\'s square into the favicon.',
       split: 'Monospace; "every" regular, "graph" bold.',
       faces: [['Text', 'Inter', 'SIL OFL 1.1', '--font-body'], ['Numbers, code, labels', 'JetBrains Mono', 'SIL OFL 1.1', '--font-mono']] },
  2: { primary: 'light', fav: 'nest',
       why: 'The same instrument, at room temperature: warm paper and soft black instead of pure white and black, an ink-blue accent that cannot be mistaken for an error, 3 px corners, labels in sentence case. Monospace is kept for what it is good at: numbers and code.',
       mistake: 'If the project wants to look sharp and a little severe — this one is polite. On a cold, blue-tinted laptop screen the paper tone can read as "slightly dirty white", and an ink-blue accent is common too (less than orange-on-black, but not rare).',
       mark: 'Medium rings in soft black, the dot in ink blue.',
       split: 'Sans; "every" regular and lighter in tone, "graph" bold.',
       faces: [['Text', 'Source Sans 3', 'SIL OFL 1.1', '--font-body'], ['Numbers, code', 'Source Code Pro', 'SIL OFL 1.1', '--font-mono']] },
  3: { primary: 'dark', fav: 'nest',
       why: 'Dark first, because the map is dark: deep ink-blue sheets, drawing-line cyan, and one warm highlight reserved for "the thing you selected". Grid, corner ticks and dimension lines are borrowed from technical drawings and used only around figures.',
       mistake: 'If most people will meet Everygraph in a white notebook or a printed paper: the system is at its best dark, and its light theme, though usable, is the least distinctive of the five. The drafting motifs become kitsch quickly if a second designer adds more of them.',
       mark: 'Thin drawing-line rings in cyan, the dot in the warm highlight.',
       split: 'Monospace, one weight; "graph" takes the accent colour.',
       faces: [['Text', 'IBM Plex Sans', 'SIL OFL 1.1', '--font-body'], ['Numbers, code, labels', 'IBM Plex Mono', 'SIL OFL 1.1', '--font-mono']] },
  4: { primary: 'light', fav: 'nest',
       why: 'B\'s discipline with an atlas\'s manners: blue-black ink on near-white, a muted sea blue for the one action on screen, hairline rules instead of boxes, and a serif for large headlines and italic captions only. Contour lines appear once per page, quietly.',
       mistake: 'If "instrument" matters more than "reference work": numbers are set in the text face (tabular) rather than monospace, and hairlines with a serif can look like a journal, not a tool. It needs the proposed serif to be shipped; on fallback fonts the headlines change character from machine to machine.',
       mark: 'Hairline rings in ink, the dot in sea blue. Needs a heavier cut below 24 px.',
       split: 'Serif; "every" in italic, "graph" in semibold roman.',
       faces: [['Headlines, captions', 'Source Serif 4', 'SIL OFL 1.1', '--font-display'], ['Text, numbers', 'Inter', 'SIL OFL 1.1', '--font-body'], ['Code', 'JetBrains Mono', 'SIL OFL 1.1', '--font-mono']] },
  5: { primary: 'light', fav: 'seal',
       why: 'As if every screen were a figure in a paper: pure black on white, no colour anywhere in the interface, so that every colour you see is data. States are told apart by weight, rule and hatching, the way a black-and-white plot does it.',
       mistake: 'As the everyday interface. Without colour an error does not jump out — it has to be read — and the page is harsh over a long session. It is a strong option for exported figures, the paper and the documentation of methods, less so for the viewer.',
       mark: 'One colour only: black rings, black dot. Survives a photocopier.',
       split: 'Bold grotesque, one weight, no split.',
       faces: [['Text, numbers', 'Arimo', 'Apache 2.0, metric-compatible with Arial', '--font-body'], ['Code', 'Cousine', 'Apache 2.0', '--font-mono']] },
};
const VAR_NAMES = { nest: 'Nest', strata: 'Strata', seal: 'Seal' };
const VAR_DESC = {
  nest: 'two rings and a dot, drifting down-left — A\'s mark',
  strata: 'three rings and a dot, drifting up-right; thinner rings',
  seal: 'the nest cut out of a square',
};

// ---------- colour custom properties from the token files ----------
(function injectTokens() {
  const kebab = k => k.replace(/[A-Z]/g, m => '-' + m.toLowerCase());
  let css = '';
  for (const id of IDS) for (const th of ['light', 'dark']) {
    const t = TOKENS[id][th], p = PAL[id];
    const v = Object.entries(t).map(([k, x]) => `--${kebab(k)}:${x}`);
    if (!t.hl) v.push('--hl:var(--accent)', '--on-hl:var(--on-accent)');
    if (id === '3' && th === 'light') v.push('--hl-ink:#8a5a00');
    p.cat[th].forEach((c, i) => v.push(`--c${i + 1}:${c}`));
    p.seq.forEach((c, i) => v.push(`--s${i + 1}:${c}`));
    p.div[th].forEach((c, i) => v.push(`--d${i + 1}:${c}`));
    css += `.eg[data-sys="${id}"][data-theme="${th}"]{${v.join(';')}}\n`;
  }
  const s = document.createElement('style'); s.textContent = css; document.head.appendChild(s);
})();

// ---------- icons ----------
const IC = {
  search: '<svg class="ic" viewBox="0 0 16 16"><circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/></svg>',
  chev: '<svg class="ic chev" viewBox="0 0 16 16"><path d="M4 6l4 4 4-4"/></svg>',
  info: '<svg class="ic" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.3"/><path d="M8 7.2v4M8 4.8v.2"/></svg>',
  warn: '<svg class="ic" viewBox="0 0 16 16"><path d="M8 2 14.5 13.5h-13z"/><path d="M8 6.5v3.2M8 11.6v.1"/></svg>',
  err: '<svg class="ic" viewBox="0 0 16 16"><path d="M5.3 1.7h5.4l3.6 3.6v5.4l-3.6 3.6H5.3l-3.6-3.6V5.3z"/><path d="M5.8 5.8l4.4 4.4M10.2 5.8l-4.4 4.4"/></svg>',
  ok: '<svg class="ic" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.3"/><path d="M5 8.2l2.1 2.1L11 6.2"/></svg>',
};

// ---------- the mark ----------
// Drawn in a 64-unit box. Three variations; each has a large (>=48 px), medium and small (<24 px) cut,
// because rings that work at 64 px close up at 16. Colours come from the surrounding .eg scope.
const MARK_W = { // ring weight per system: [nest L, M, S], [strata L, M, S], [seal L, M, S]
  1: { nest: [5, 5.6, 7.6], strata: [3.6, 4.2, 6.6], seal: [4.4, 4.8, 6.4], rx: 0 },
  2: { nest: [3.8, 4.6, 7.2], strata: [2.8, 3.6, 6.2], seal: [3.6, 4.2, 6.2], rx: 7 },
  3: { nest: [2.4, 3.6, 6.6], strata: [1.8, 3.1, 6], seal: [2.4, 3.4, 5.8], rx: 0 },
  4: { nest: [2, 3.3, 6.2], strata: [1.6, 3, 5.8], seal: [2, 3.2, 5.6], rx: 1.5 },
  5: { nest: [6.4, 6.8, 8.2], strata: [4.2, 4.6, 7], seal: [5, 5.4, 6.8], rx: 0 },
};
function nestGeom(R, w, cx, cy, small) { // R: outer edge radius
  const u = [-0.75, 0.66], ro = R - w / 2, e0 = ro - w / 2;
  if (small) { const rd = e0 * 0.44, off = e0 - rd - Math.max(4.6, w * 0.62); return { rings: [[cx, cy, ro]], dot: [cx + u[0] * off, cy + u[1] * off, rd] }; }
  const g = w < 3.5 ? 2 : Math.max(1.3, w * 0.25), ri = e0 * (w < 3.5 ? 0.6 : 0.56), o1 = e0 - ri - w / 2 - g, e1 = ri - w / 2;
  const rd = e1 * 0.5, o2 = e1 - rd - (w < 3.5 ? 2.2 : Math.max(1.6, w * 0.3));
  return { rings: [[cx, cy, ro], [cx + u[0] * o1, cy + u[1] * o1, ri]], dot: [cx + u[0] * (o1 + o2), cy + u[1] * (o1 + o2), rd] };
}
function strataGeom(R, w, small) {
  const u = [0.77, -0.64], ro = R - w / 2, e0 = ro - w / 2, c = 32;
  if (small) { const r2 = e0 * 0.5, o = e0 - r2 - w / 2 - Math.max(2.6, w * 0.4); return { rings: [[c, c, ro], [c + u[0] * o, c + u[1] * o, r2]], dot: null }; }
  const g = w < 3 ? 1.8 : Math.max(1.3, w * 0.3), r2 = e0 * 0.66, o1 = e0 - r2 - w / 2 - g, e1 = r2 - w / 2;
  const r3 = e1 * 0.6, o2 = e1 - r3 - w / 2 - g, e2 = r3 - w / 2, rd = e2 * 0.44, o3 = e2 - rd - g;
  return { rings: [[c, c, ro], [c + u[0] * o1, c + u[1] * o1, r2], [c + u[0] * (o1 + o2), c + u[1] * (o1 + o2), r3]], dot: [c + u[0] * (o1 + o2 + o3), c + u[1] * (o1 + o2 + o3), rd] };
}
function egMark(sys, variant, px) {
  const cls = px >= 48 ? 0 : px >= 24 ? 1 : 2, W = MARK_W[sys], w = W[variant][cls], f = n => +n.toFixed(2);
  const a = `width="${px}" height="${px}" viewBox="0 0 64 64" role="img" aria-label="Everygraph mark"`;
  const ring = (r, col) => `<circle cx="${f(r[0])}" cy="${f(r[1])}" r="${f(r[2])}" fill="none" stroke="${col}" stroke-width="${w}"/>`;
  const dot = (d, col) => d ? `<circle cx="${f(d[0])}" cy="${f(d[1])}" r="${f(d[2])}" fill="${col}"/>` : '';
  if (variant === 'seal') {
    const g = nestGeom(cls === 2 ? 24.5 : 24, w, 32, 32, cls === 2);
    const border = sys == 3 ? ' stroke="var(--line-strong)" stroke-width="1" vector-effect="non-scaling-stroke"' : '';
    return `<svg ${a}><rect x="${border ? .5 : 0}" y="${border ? .5 : 0}" width="${border ? 63 : 64}" height="${border ? 63 : 64}" rx="${W.rx}" fill="var(--mark-sq)"${border}/>${g.rings.map(r => ring(r, 'var(--mark-on-sq)')).join('')}${dot(g.dot, 'var(--mark-sq-dot)')}</svg>`;
  }
  const g = variant === 'nest' ? nestGeom(31, w, 32, 32, cls === 2) : strataGeom(31, w, cls === 2);
  return `<svg ${a}>${g.rings.map(r => ring(r, 'var(--mark-ring)')).join('')}${dot(g.dot, 'var(--mark-dot)')}</svg>`;
}
// Wordmark as live SVG text (a mockup: the chosen wordmark would be drawn outlines).
function egWordmark(sys, h) {
  const fs = h * 0.8, y = h * 0.74; let t;
  if (sys == 1) t = `<text x="0" y="${y}" font-size="${fs * 0.86}" style="font-family:var(--font-mono);fill:var(--ink);letter-spacing:-0.04em"><tspan font-weight="400">every</tspan><tspan font-weight="700">graph</tspan></text>`;
  else if (sys == 2) t = `<text x="0" y="${y}" font-size="${fs}" style="font-family:var(--font-display);letter-spacing:-0.02em"><tspan font-weight="400" style="fill:var(--ink2)">every</tspan><tspan font-weight="700" style="fill:var(--ink)">graph</tspan></text>`;
  else if (sys == 3) t = `<text x="0" y="${y}" font-size="${fs * 0.8}" style="font-family:var(--font-mono);letter-spacing:0.01em" font-weight="400"><tspan style="fill:var(--ink)">every</tspan><tspan style="fill:var(--accent-ink)">graph</tspan></text>`;
  else if (sys == 4) t = `<text x="0" y="${y}" font-size="${fs * 1.04}" style="font-family:var(--font-display);fill:var(--ink);letter-spacing:-0.01em"><tspan font-style="italic" font-weight="400">every</tspan><tspan font-weight="700" dx="${fs * 0.03}">graph</tspan></text>`;
  else t = `<text x="0" y="${y}" font-size="${fs}" style="font-family:var(--font-display);fill:var(--ink);letter-spacing:-0.045em" font-weight="700">everygraph</text>`;
  return `<svg class="eg-wm" height="${h}" role="img" aria-label="everygraph" style="overflow:visible">${t}</svg>`;
}
function egLockup(sys, h, variant) {
  return `<span style="display:inline-flex;align-items:center;gap:${Math.round(h * 0.3)}px;line-height:0">${egMark(sys, variant || SYS[sys].fav, Math.round(h * 0.95))}${egWordmark(sys, h)}</span>`;
}
function fitWordmarks(root) {
  (root || document).querySelectorAll('svg.eg-wm').forEach(s => {
    const b = s.querySelector('text').getBBox(), h = +s.getAttribute('height');
    s.setAttribute('width', Math.ceil(b.width + 1)); s.setAttribute('viewBox', `${b.x} 0 ${Math.ceil(b.width + 1)} ${h}`);
  });
}

// ---------- small generators ----------
function rng(seed) { let s = seed >>> 0; return () => { s = s + 0x6D2B79F5 | 0; let t = Math.imul(s ^ s >>> 15, 1 | s); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; }
// A tiny fake map: twelve communities with their sub-communities, coloured by the data palette.
const MINIMAP = (() => {
  const R = rng(5), W = 640, H = 150, radii = [60, 50, 43, 38, 34, 31, 28, 26, 24, 22, 20, 18], top = [];
  for (const r of radii) for (let k = 0; k < 4000; k++) {
    const x = r + 3 + R() * (W - 2 * r - 6), y = r + 3 + R() * (H - 2 * r - 6);
    if (top.every(c => Math.hypot(c.x - x, c.y - y) > c.r + r + 4)) { top.push({ x, y, r }); break; }
  }
  top.sort((a, b) => a.x - b.x);
  let s = '';
  top.forEach((c, i) => {
    const col = `var(--c${i % 10 + 1})`, kids = [];
    const n = 2 + Math.floor(R() * 5);
    for (let j = 0; j < n; j++) for (let k = 0; k < 200; k++) {
      const r = c.r * (0.14 + R() * 0.34), a = R() * 6.283, d = R() * (c.r - r - 3), x = c.x + Math.cos(a) * d, y = c.y + Math.sin(a) * d;
      if (kids.every(q => Math.hypot(q.x - x, q.y - y) > q.r + r + 2)) { kids.push({ x, y, r }); break; }
    }
    s += `<circle cx="${c.x.toFixed(1)}" cy="${c.y.toFixed(1)}" r="${c.r}" fill="${col}" fill-opacity=".07" stroke="${col}" stroke-width="1.4"/>`;
    kids.forEach(q => { s += `<circle cx="${q.x.toFixed(1)}" cy="${q.y.toFixed(1)}" r="${q.r.toFixed(1)}" fill="${col}" fill-opacity=".1" stroke="${col}" stroke-width="1.1"/>`;
      if (q.r > 9) s += `<circle cx="${(q.x - q.r * .25).toFixed(1)}" cy="${(q.y + q.r * .2).toFixed(1)}" r="${(q.r * .38).toFixed(1)}" fill="none" stroke="${col}" stroke-width="1"/>`; });
  });
  return `<svg viewBox="0 0 ${W} ${H}" width="100%" role="img" aria-label="Fake map coloured with the data palette">${s}</svg>`;
})();
// Contour lines (Survey's motif): nested, slightly irregular closed curves.
function contours(size, n, seed, stroke) {
  const R = rng(seed), ph = [R() * 6.28, R() * 6.28, R() * 6.28]; let s = '';
  for (let k = 0; k < n; k++) {
    const t = k / n, r = size * 0.48 * (1 - t * 0.86), cx = size / 2 + t * size * 0.13, cy = size / 2 - t * size * 0.09, pts = [];
    for (let i = 0; i <= 96; i++) { const a = i / 96 * 6.2832, rr = r * (1 + 0.09 * Math.sin(a + ph[0]) + 0.06 * Math.sin(2 * a + ph[1] + t * 2) + 0.035 * Math.sin(3 * a + ph[2] - t * 3)); pts.push(`${(cx + Math.cos(a) * rr).toFixed(1)} ${(cy + Math.sin(a) * rr).toFixed(1)}`); }
    s += `<path d="M${pts.join('L')}Z" fill="none" stroke="${stroke}" stroke-width="${k % 4 === 0 ? 1.2 : 0.7}" vector-effect="non-scaling-stroke"/>`;
  }
  return `<svg class="contours" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}" aria-hidden="true">${s}</svg>`;
}
// Which family of a CSS font stack is actually installed here (so the page can say what you are looking at).
function renderedFont(el, prop) {
  const stack = getComputedStyle(el).getPropertyValue(prop).split(',').map(s => s.trim().replace(/^["']|["']$/g, '')).filter(Boolean);
  const c = document.createElement('canvas').getContext('2d'), probe = 'mmmmmmmmlli 0123456789 WgQ@ fi';
  const w = f => { c.font = '72px ' + f; return c.measureText(probe).width; };
  const base = [w('monospace'), w('serif'), w('sans-serif')];
  for (const f of stack) {
    const generic = /^(system-ui|ui-[a-z-]+|-apple-system|BlinkMacSystemFont)$/.test(f), q = generic ? f : `"${f}"`;
    if (/^(serif|sans-serif|monospace)$/.test(f)) return { name: 'the browser\'s generic ' + f, first: false };
    if (w(q + ',monospace') !== base[0] || w(q + ',serif') !== base[1] || w(q + ',sans-serif') !== base[2]) return { name: generic ? 'the system default (' + f + ')' : f, first: f === stack[0] };
  }
  return { name: 'the browser default', first: false };
}

// ---------- invented content ----------
const ROWS = [['0', '31,204,118', '412,880,341', '96,412,077', '14.69 %'], ['1', '22,870,455', '268,114,902', '71,330,518', '10.77 %'], ['2', '18,441,002', '201,557,310', '58,902,144', '8.68 %'], ['3', '9,112,876', '97,340,221', '30,118,406', '4.29 %']];
const numTable = () => `<table class="nt"><thead><tr><th>Community</th><th>Addresses</th><th>Transfers inside</th><th>Transfers out</th><th>Share</th></tr></thead><tbody>${ROWS.map((r, i) => `<tr${i === 1 ? ' class="sel"' : ''}><td><i class="sw" style="background:var(--c${i + 1})"></i>${r[0]}</td>${r.slice(1).map(c => `<td>${c}</td>`).join('')}</tr>`).join('')}<tr><td class="dim" colspan="5">… 31,956 more · sorted by addresses, largest first</td></tr></tbody></table>`;
const codeBlock = sys => { const L = [`<span class="k">import</span> everygraph <span class="k">as</span> eg`, `eg.config(memory=<span class="s">"8GB"</span>)            <span class="c"># a hard limit</span>`, `g = eg.read(<span class="s">"transfers.parquet"</span>)   <span class="c"># 1.83 B edges</span>`, `comm = g.communities(seed=0)`]; return `<pre class="code">${sys == 3 ? L.map(l => `<span class="ln">${l}</span>`).join('') : L.join('\n')}</pre>`; };
const callout = (kind, tag, title, text) => `<div class="co ${kind}">${IC[kind]}<span class="tag">${tag}</span><div class="body"><b>${title}</b> ${text}</div></div>`;
const tooltip = (cls = '') => `<div class="tip ${cls}"><b>Community 301832</b><dl><dt>nodes</dt><dd>20,064</dd><dt>edges inside</dt><dd>124,356</dd><dt>edges leaving</dt><dd>885,331</dd><dt>level</dt><dd>0 of 6</dd></dl></div>`;
const progress = sys => `<div class="prog"><div class="prog-top"><b>Building transfers.eg</b><span class="num">1,270 of 2,048 parts</span><span class="eta">about 22 s left</span></div>
<div class="prog-bar"><i class="done" style="flex:10"></i><i class="done" style="flex:5"></i><i class="cur" style="flex:7"><b style="width:62%"></b></i><i class="todo" style="flex:4"></i></div>${sys == 2 ? '<div class="prog-ticks"></div>' : ''}
<div class="prog-lbl"><span style="flex:10">reading<em>1 min 41 s</em></span><span style="flex:5">identifiers<em>38 s</em></span><span class="cur" style="flex:7">building<em>62 %</em></span><span class="todo" style="flex:4">checking<em>~12 s</em></span></div></div>`;
const legend = () => `<div class="legend"><span class="lbl">Colour · community</span>${[['Community 0', '31.2 M'], ['Community 1', '22.9 M'], ['Community 2', '18.4 M'], ['Community 3', '9.1 M']].map((r, i) => `<div class="row"><i class="sw" style="--c:var(--c${i + 1})"></i>${r[0]}<span class="num">${r[1]}</span></div>`).join('')}<div class="row" style="color:var(--muted)"><i class="sw" style="--c:var(--muted)"></i>31,956 more</div><hr><span class="lbl">Shade · transfers inside</span><div class="ramp flip">${[1, 2, 3, 4, 5, 6, 7].map(i => `<i style="background:var(--s${i})"></i>`).join('')}</div><div class="ends"><span class="num">10³</span><span class="num">10⁸</span></div></div>`;
const emptyState = sys => `<div class="empty"><div class="art">${sys == 4 ? contours(92, 7, 3, 'var(--line-strong)').replace('class="contours"', '') : `<svg width="84" height="84" viewBox="0 0 64 64" aria-hidden="true"><g fill="none" stroke="var(--line-strong)" stroke-width="1.2" stroke-dasharray="${sys == 5 ? '0' : '3 3'}"><circle cx="32" cy="32" r="29"/><circle cx="25" cy="38" r="17"/></g><circle cx="20" cy="42" r="5" fill="var(--line-strong)"/></svg>`}</div><div><h3>No graph open yet</h3><p>Drop a CSV, Parquet or edge-list file here. You will see what was read before anything is computed.</p><div class="acts"><span class="btn sm">Choose a file</span><span class="btn sm sec">Open a sample map</span><code>everygraph open transfers.eg</code></div></div></div>`;

const TERM = `<span class="p">$</span> everygraph ingest transfers.parquet --source from --target to \\
      --weight value --time block_time
<span class="hd">transfers.parquet</span> · 38.2 GB · 1,830,402,118 rows

  <span class="d">ids</span>       hex addresses (20 bytes) · ~212.4 M distinct
  <span class="d">edges</span>     directed · duplicates kept as events · 4,201 self-loops dropped
  <span class="d">columns</span>   value → weight · block_time → time · ignored: gas, nonce, input

  <span class="d">machine</span>   16 cores · 61 GB RAM (44 GB free) · NVMe, 1.4 TB free
  <span class="d">budget</span>    8.0 GB memory (default: a fifth of free RAM, at least 2 GB)
  <span class="d">plan</span>      build on disk — the graph is 4.6× the memory budget
  <span class="d">estimate</span>  3 min 40 s · peak 5.9 GB · 61 GB scratch in /tmp · result 9.3 GB

  reading       <span class="b"></span>  1,830,402,118 rows      <span class="d">1 min 41 s</span>
  identifiers   <span class="b"></span>  212,418,907 addresses        <span class="d">38 s</span>
  building      <span class="b"></span>  2,048 parts                  <span class="d">59 s</span>
  checking      <span class="b"></span>  sorted · symmetric counts    <span class="d">12 s</span>

<span class="ok">✓ transfers.eg</span> · 9.3 GB · 212,418,907 nodes · 1,830,397,917 edges · 3 min 30 s
  <span class="d">next:</span> <span class="nx">everygraph map transfers.eg</span>`;

// ---------- blocks ----------
const pane = (sys, th, inner, cls = 'pane') => `<div class="eg ${cls}" data-sys="${sys}" data-theme="${th}">${inner}</div>`;
const themes = sys => SYS[sys].primary === 'dark' ? ['dark', 'light'] : ['light', 'dark'];

function overviewCard(sys) {
  const s = SYS[sys], t = TOKENS[sys], th = s.primary, alt = th === 'light' ? 'dark' : 'light', k = t[th];
  const acc = `<div class="acc"><i style="background:${k.accent}"></i>${k.hl ? `<i style="background:${k.hl}"></i>` : ''}<span>${sys == 5 ? 'no accent' : 'accent' + (k.hl ? ' + highlight' : '')}</span><span class="sp"></span>${sys == 5 ? '<span>status by shape</span>' : ['ok', 'warn', 'err', 'info'].map(x => `<i class="st" style="background:${k[x]}"></i>`).join('')}</div>`;
  return `<a class="card" href="#only=${sys}" style="text-decoration:none">
  <div class="eg top" data-sys="${sys}" data-theme="${th}">
    <div class="hd"><i>${sys}</i><b style="color:var(--ink)">${t.name}</b><span>${th}-first</span></div>
    <div class="lock">${egLockup(sys, 34)}</div>
    <div class="why">${t.tag.charAt(0).toUpperCase() + t.tag.slice(1)}. ${s.short}</div>
    ${acc}
    <div class="dots">${PAL[sys].cat[th].map(c => `<i style="--c:${c}"></i>`).join('')}</div>
    <div class="bigno"><span class="num">1,830,397,917</span><small>edges, exactly</small></div>
    <div class="rowb"><span class="btn ${sys == 1 ? 'accent' : ''}">Open a graph</span><span class="btn quiet">Explain plan</span></div>
  </div>
  <div class="eg alt" data-sys="${sys}" data-theme="${alt}">${egLockup(sys, 22)}<span class="btn sm ${sys == 1 ? 'accent' : ''}">Run</span></div></a>`;
}
SYS[1].short = 'The reference point: square, black, one orange, uppercase mono labels.';
SYS[2].short = 'Warm paper, soft black, ink-blue accent, sentence-case labels, 3 px corners.';
SYS[3].short = 'Ink-blue sheets, cyan lines, one warm highlight, drafting motifs. The map looks native.';
SYS[4].short = 'Hairlines, a serif for headlines and captions, a muted sea blue, contour lines.';
SYS[5].short = 'Black on white, no accent at all. Every colour on screen is data.';

function headCard(sys, th) {
  const s = SYS[sys], t = TOKENS[sys];
  return `<div class="card sys-headcard">${pane(sys, th, `
    <div><div class="lk">${egLockup(sys, 46)}</div><p class="tagline">${t.tag}</p></div>
    <div><span class="lbl">Rationale</span><p>${s.why}</p></div>
    <div><span class="lbl">Where this would be a mistake</span><p>${s.mistake}</p></div>`, 'sys-head')}</div>`;
}
function identityPane(sys, th) {
  const s = SYS[sys];
  const v = ['nest', 'strata', 'seal'].map(k => `<div class="var"><div class="sizes">${[64, 32, 16].map(px => egMark(sys, k, px)).join('')}</div>
     <div class="cap"><b>${VAR_NAMES[k]}</b>${s.fav === k ? ' <span class="fav">· favourite</span>' : ''}<br>${VAR_DESC[k]}</div></div>`).join('');
  return pane(sys, th, `<div class="csec"><span class="lbl">Mark · three variations at 64, 32 and 16 px · on ${th}</span><div class="vars">${v}</div></div>
   <div class="locks"><div class="h">${egLockup(sys, 50)}</div><div class="st">${egMark(sys, s.fav, 54)}${egWordmark(sys, 26)}</div></div>
   <div class="idrow"><div class="minis"><span>Small lockup</span>${egLockup(sys, 20)}</div>
     <div class="btabs"><span class="btab">${egMark(sys, s.fav, 16)} transfers — everygraph <span class="x">×</span></span><span class="btab off">docs.everygraph.dev</span></div></div>`);
}
function colourPane(sys, th) {
  const t = TOKENS[sys][th], p = PAL[sys], sep = p.sep[th], st = p.status[th];
  const sw = (name, hex) => `<div class="swt"><i style="background:${hex}"></i>${name}<code>${hex}</code></div>`;
  const surf = [['page', t.bg], ['surface', t.surface], ['inset', t.surface2], ['rule', t.line], ['muted', t.muted], ['text 2', t.ink2], ['text', t.ink], ['accent', t.accent], t.hl ? ['highlight', t.hl] : ['accent text', t.accentInk]];
  const status = [['ok', 'success'], ['warn', 'warning'], ['err', 'error'], ['info', 'info']].map(([k, n]) => `<div class="swt"><i style="background:${t[k]}">${IC[k]}</i>${n}<code>${t[k]}</code></div>`).join('');
  const closest = Object.entries(st).sort((a, b) => Math.min(a[1].normal, a[1].protan, a[1].deutan) - Math.min(b[1].normal, b[1].protan, b[1].deutan))[0];
  const nm = { ok: 'success', warn: 'warning', err: 'error', info: 'info' };
  const proofNote = sys == 5
    ? `<b>No colour here by design.</b> Accent and status are the same black; they differ by shape — a heavy left bar and an inverted word for errors, a dashed rule for warnings, a plain hairline for notes. Whether that is enough is the question this system asks.`
    : `<b>Accent vs error: ΔE ${st.err.normal}</b> with normal vision, ${st.err.protan} protanopia, ${st.err.deutan} deuteranopia. Closest status to the accent: ${nm[closest[0]]}, ΔE ${Math.min(closest[1].normal, closest[1].protan, closest[1].deutan)} at worst.${sys == 1 ? ` <b>This is the weakness round 2 is meant to fix.</b> ${th === 'dark' ? 'Round 1 used one red for both themes (3.6:1 on this page); the status colours here are lightened to be legible, which moves the red next to the orange.' : 'B defined no info colour; the blue is added here.'}` : ''}${sys == 3 ? ` Info is drawn in the text colour, not a hue. Highlight vs warning: ΔE ${p.hl[th].vsWarn}.` : ''}${sys == 4 ? ' Info is drawn in the text colour, not a hue.' : ''}`;
  const worst = `<b>Worst of all 45 pairs: ΔE ${sep.cvd}</b> under protanopia and deuteranopia (target ≥ 8), ${sep.normal} with normal vision, ${sep.tritan} under tritanopia. ${sep.below3 ? `${sep.below3} of the 10 fall below 3:1 against this surface: fills with labels, not hairlines.` : 'All 10 reach 3:1 against this surface, so they hold as thin outlines.'}${sys == 1 ? ' Round 1\'s palette, unchanged.' : ''}${sys == 5 ? ' One palette for both surfaces, so a figure prints as it looks on screen.' : ''}`;
  return pane(sys, th, `
   <div class="csec"><span class="lbl">Surfaces, text and accent · ${th}</span><div class="sw-row">${surf.map(x => sw(x[0], x[1])).join('')}</div></div>
   <div class="csec"><span class="lbl">Status, next to the accent</span><div class="status-row"><div><div class="sts">${status}</div></div>
     <div class="proof"><span class="btn ${sys == 1 ? 'accent' : ''}">Run</span>${callout('err', 'Error', 'Not enough memory.', 'Nothing was changed.')}</div></div>
     <p class="note">${proofNote}</p></div>
   <div class="csec"><span class="lbl">Data palette · 10 categories, fixed order</span><div class="data-row">${p.cat[th].map(c => `<div class="swt"><i style="background:${c}"></i><code>${c}</code></div>`).join('')}</div>
     <div class="cvd">${p.sim[th].protan.map(c => `<i style="background:${c}"></i>`).join('')}</div><div class="cvd">${p.sim[th].deutan.map(c => `<i style="background:${c}"></i>`).join('')}</div>
     <div class="cvdlab"><span>thin strips: the same ten as seen with protanopia (upper) and deuteranopia (lower), simulated</span></div>
     <div class="mapdemo">${MINIMAP}</div><p class="note">${worst}</p></div>
   <div class="csec ramps"><div><span class="lbl">Sequential · low to high</span><div class="seq ramp flip">${p.seq.map(c => `<i style="background:${c}"></i>`).join('')}</div><div class="ends"><span>10³</span><span>10⁸</span></div></div>
     <div><span class="lbl">Diverging · around a neutral middle</span><div class="seq">${p.div[th].map(c => `<i style="background:${c}"></i>`).join('')}</div><div class="ends"><span>fewer than expected</span><span>more</span></div></div></div>`);
}
function typeBlock(sys, th) {
  const s = SYS[sys];
  return `<div class="card eg type" data-part="type" data-sys="${sys}" data-theme="${th}"><div>
    <div class="ln"><span>Display 44</span><span class="display" style="font-size:44px">Every graph, every size</span></div>
    <div class="ln"><span>Title 28</span><span class="display" style="font-size:28px">Never run out of memory</span></div>
    <div class="ln"><span>Heading 19</span><span class="display" style="font-size:19px">What the plan says before it runs</span></div>
    <div class="ln"><span>Body 15</span><span style="font-size:15px">You set a budget; the engine plans first and stays inside it.</span></div>
    <div class="ln"><span>Small 13</span><span style="font-size:13px;color:var(--ink2)">Invented example: 212.4 M addresses and 1.83 B transfers.</span></div>
    <div class="ln"><span>Label</span><span><span class="lbl" style="display:inline-flex;min-width:300px">Largest communities</span></span></div>
    <div class="ln"><span>Numbers 15</span><span class="num" style="font-size:calc(15px*var(--num-size) / 1em)">1,830,397,917 · 212,418,907 · 0.603 · 9.3 GB</span></div>
    <div class="ln"><span>Code 13</span><code style="font-size:13px">g.explain("communities")</code></div></div>
   <div class="run"><h3>See it and trust it</h3>
    <p>A zoomable map of the whole graph, with exact numbers at every level. The largest community holds <span class="num">31,204,118</span> addresses, <span class="num">14.69 %</span> of the graph; open it and it splits into <span class="num">1,206</span> smaller ones.</p>
    <p style="font-size:14px"><a href="#">How the map is built</a> · <a href="#">What "exact" means here</a></p>
    <div class="faces">${s.faces.map(f => `<div data-face="${f[3]}"><span>${f[0]}</span><span>proposed <b>${f[1]}</b> (${f[2]}) — <span class="seen"></span></span></div>`).join('')}
      <div><span>Rule</span><span>${s.typeRule}</span></div></div></div></div>`;
}
SYS[1].typeRule = 'Labels, buttons and table heads in uppercase monospace; every number in monospace.';
SYS[2].typeRule = 'Everything in sentence case. Monospace only for numbers and code. Two weights: regular and bold.';
SYS[3].typeRule = 'Headlines in medium weight, never bold. Labels in lowercase monospace with a leader line. Numbers in monospace.';
SYS[4].typeRule = 'Serif for headlines of 19 px and up and for italic captions and labels; nothing else. Numbers in the text face with tabular figures; monospace only for code.';
SYS[5].typeRule = 'One family, two weights. Bold does the work colour does elsewhere. Numbers in the text face with tabular figures.';

function compsBlock(sys, th) {
  const box = sys == 4 ? 'boxed' : '';
  return `<div class="card eg comps" data-part="components" data-sys="${sys}" data-theme="${th}">
   <div>
    <div><span class="lbl">Text input, search, select</span>
     <div class="rowf"><div class="field grow"><span class="k">Memory budget</span><span class="val">8 GB</span></div><div class="field grow focus"><span class="val">transfers.parquet</span><span class="caret"></span></div><div class="field select ${box}" style="width:200px"><span class="k">Colour:</span> community ${IC.chev}</div></div>
     <div class="rowf"><div class="field grow">${IC.search}<span class="ph">Find an address or a community, e.g. 0x7a3f…</span><kbd>/</kbd></div><div class="field select ${box}" style="width:200px"><span class="k">Size:</span> addresses ${IC.chev}</div></div></div>
    <div><span class="lbl">Progress with phases</span>${progress(sys)}</div>
    <div><span class="lbl">Table of exact numbers</span>${numTable()}</div>
    <div class="stack"><span class="lbl">Callouts</span>
     ${callout('info', 'Note', 'This step streams edges from disk:', 'about one sequential read per pass.')}
     ${callout('warn', 'Warning', '4,201 self-loops were dropped.', 'Keep them with <code>--keep-loops</code>.')}
     ${callout('err', 'Error', 'Not enough memory for this step.', 'The budget is 4.0 GB; 5.6 GB is the smallest that works.')}</div>
   </div>
   <div>
    <div><span class="lbl">Buttons · primary, secondary, quiet, disabled</span><div class="rowf"><span class="btn">Open a graph</span>${sys == 1 ? '<span class="btn accent">Run</span>' : ''}<span class="btn sec">Explain plan</span><span class="btn quiet">Cancel</span><span class="btn" disabled><span>Export</span></span></div></div>
    <div><span class="lbl">Toggle and tabs</span><div class="rowf" style="gap:18px"><span class="toggle on"><i></i>Draw edges</span><span class="toggle"><i></i>Labels</span></div>
     <div class="tabs" style="margin-top:12px"><span class="on">Communities</span><span>Top nodes</span><span>Neighbours</span><span>Plan</span></div></div>
    <div><span class="lbl">Code</span>${codeBlock(sys)}</div>
    <div class="split"><div><span class="lbl">Tooltip</span><div class="tipwrap">${tooltip()}</div></div><div><span class="lbl">Legend</span>${legend()}</div></div>
    <div><span class="lbl">Empty state</span>${emptyState(sys)}</div>
   </div>
  </div>`;
}

function hero(sys) {
  return `<div class="ctx-cap"><b>a</b> Landing page, first screen. The picture is a real screenshot from the lab prototype.</div>
  <div class="hero">
   <nav>${egLockup(sys, 24)}<span class="soon">design mockup — nothing to install yet</span><span class="sp"></span><a>Docs</a><a>Gallery of maps</a><a>Measurements</a><a>Roadmap</a><a>GitHub</a></nav>
   ${sys == 4 ? contours(520, 13, 9, 'var(--line)') : ''}
   <div class="grid"><div>
     <span class="lbl">Open-source graph engine · one machine</span>
     <h1>Every graph, every size, <u>every machine.</u></h1>
     <p class="lead">One engine to load, measure, query and map a graph — from a thousand edges to billions — on a single computer, inside the memory you give it.</p>
     <div class="install"><span class="p">$</span> pip install everygraph<span class="cp">Copy</span></div>
     <div class="cta"><span class="btn">Get started in 5 minutes</span><span class="btn sec">Open a sample map</span></div>
     <p class="fine">Also: <code>cargo add everygraph</code> · a single binary for the command line · Apache-2.0 or MIT</p></div>
    <div><div class="shot ticks"><img src="../../../docs/img/map-overview-orkut.png" alt="Map of com-Orkut from the lab prototype"><div class="hud"><b>com-Orkut</b><span><span class="num">3,072,441</span> nodes · <span class="num">117,185,083</span> edges</span></div></div>
     <div class="dimline"><span>3,072,441 nodes · 117,185,083 edges · zoom 1.9</span></div>
     <div class="ruler"><div class="t"></div><i class="m" style="left:21.1%"></i>${[0, 3, 6, 9].map(n => `<span class="n" style="left:${n / 9 * 100}%">${n}</span>`).join('')}<span class="n" style="left:21.1%;color:var(--accent-ink);top:10px">zoom 1.9</span></div>
     <div class="shot-cap"><span>${sys == 5 ? '<b>Figure 1 | </b>' : sys == 4 ? 'Plate 1. ' : ''}Every circle is a community; the number on it is exact. Zoom in until the circles are single nodes.</span><span>Drag · scroll · click</span></div></div>
   </div></div>`;
}
function termAndNotebook(sys) {
  const title = { 1: 'bash — 96×24', 2: 'Terminal', 3: 'tty · ingest', 4: 'At the command line', 5: 'Terminal' }[sys];
  return `<div class="pair"><div><div class="ctx-cap"><b>b</b> Command line: plan, estimate, progress (wording from the CLI preview).</div>
    <div class="term"><div class="bar"><i></i><i></i><i></i><span>${title}</span></div><pre>${TERM}</pre></div></div>
   <div><div class="ctx-cap"><b>c</b> Notebook output inside JupyterLab, which keeps its own look: “does not fit”, with two ways out.</div>
    <div class="nbhost"><div class="cell"><div class="pr">[3]:</div><div class="in">comm = g.communities()</div></div>
     <div class="cell out"><div class="pr"></div><div class="eg" data-sys="${sys}" data-theme="${document.currentTheme}" style="background:transparent">
      <div class="ecard"><div class="hd">${IC.err}<b>InsufficientBudget</b><span>communities() does not fit in the 4.0 GB budget. Nothing was run.</span></div>
      <div class="bd"><div class="budget"><div class="track"><div class="seg a" style="width:85%">5.1 GB · per-address state, must stay in memory</div><div class="seg b" style="width:8.3%"></div><div class="mark" style="left:66.7%"></div></div>
        <div class="ax"><span style="left:66.7%;transform:translateX(-100%);padding-right:6px">budget 4.0 GB</span><span style="right:0">+ 0.5 GB buffers</span></div></div>
       <p class="sm"><b>Smallest budget that works: 5.6 GB.</b> <span>24 bytes for each of <span class="num">212,418,907</span> addresses; the edges can stay on disk.</span></p>
       <div class="ways"><div class="way"><div class="t"><i>1</i>Raise the budget</div><code>eg.config(memory="6GB")</code><p>This machine has 16 GB; 11.2 GB are free right now.</p></div>
        <div class="way"><div class="t"><i>2</i>Work on part of the graph</div><code>g.since("2023-01-01").communities()</code><p><span class="num">96,118,340</span> addresses: needs 2.8 GB, fits as it is.</p></div></div>
       <p class="ft">Full plan: <code>g.explain("communities")</code> · Python traceback (1 frame) ▸</p></div></div></div></div>
     <div class="cell" style="margin-top:10px"><div class="pr">[4]:</div><div class="in">eg.config(memory="6GB")</div></div></div></div></div>`;
}
function viewer(sys) {
  const rank = [['c 4411', 1204, 1], ['c 4407', 1118, 0], ['c 4420', 902, 0], ['c 4396', 627, 0], ['c 4402', 617, 0], ['c 4431', 609, 0]];
  return `<div class="ctx-cap"><b>d</b> Map viewer: the chrome is the mockup; the map is a real lab screenshot (soc-LiveJournal), so its labels and colours are the prototype's.</div>
  <div class="viewer">
   <div class="vbar"><span class="file">${egMark(sys, SYS[sys].fav, 22)} livejournal.eg</span>
     <div class="field search">${IC.search}<span class="ph">Find a node or a community</span><kbd>/</kbd></div>
     <div class="field select ${sys == 4 ? 'boxed' : ''}"><span class="k">Colour:</span> community ${IC.chev}</div><div class="field select ${sys == 4 ? 'boxed' : ''}"><span class="k">Size:</span> nodes ${IC.chev}</div>
     <span class="toggle on"><i></i>Edges</span><span class="sp"></span>
     <div class="zoom"><span>−</span><b class="num">6.6</b><span>+</span></div><span class="btn sec sm" style="height:32px">Share</span></div>
   <div class="vmap"><img src="../../../docs/img/map-region-lj.png" alt="A region of soc-LiveJournal from the lab prototype">${tooltip('noarrow')}<span class="tag">lab prototype screenshot</span></div>
   <aside class="vside"><span class="lbl">Selected</span><h4>Community 301832</h4><span class="sub">level 0 of 6 · inside continent 0 · 0.50 % of all nodes</span>
     <div class="kv"><div><b class="num">20,064</b><span>nodes</span></div><div><b class="num">124,356</b><span>edges inside</span></div><div><b class="num">885,331</b><span>edges leaving</span></div><div><b class="num">1,134,043</b><span>sum of degrees</span></div></div>
     <div class="shown">Edges drawn at this zoom: <span class="num">52,022</span> of <span class="num">883,457</span> by weight. 57 regions (<span class="num">1,143</span> nodes) are folded.<div class="bar"><i></i></div></div>
     <div class="tabs"><span class="on">Sub-communities</span><span>Top nodes</span><span>Neighbours</span></div>
     <ol class="rank">${rank.map((r, i) => `<li${r[2] ? ' class="on"' : ''}><i class="num">${i + 1}</i><span>${r[0]}</span><em><b style="width:${r[1] / 1204 * 100}%"></b></em><span class="num">${r[1].toLocaleString('en-US')}</span></li>`).join('')}</ol>
     <div class="vacts"><span class="btn sm">Fly to</span><span class="btn sm sec">Open as table</span><span class="btn sm quiet">Copy link</span></div></aside>
   <div class="vcap"><b>soc-LiveJournal</b><span><span class="num">3,997,962</span> nodes · <span class="num">34,681,189</span> edges</span><span class="m">zoom 6.6 of 10 · 35 tiles · 1,863 KB</span><span class="sp"></span><span class="it">Every circle is a community; every number is exact.</span></div>
  </div>`;
}

function section(sys, th) {
  const t = TOKENS[sys], [a, b] = themes(sys), alt = SYS[sys].primary === 'light' ? 'dark' : 'light';
  document.currentTheme = th;
  const meta = (part, title, note = '') => `<div class="meta" data-part="${part}"><b>${title}</b><span>${note}</span></div>`;
  return `<section class="sys" id="s${sys}">
   <div class="sys-title"><span class="n">${sys}</span><h2>${t.name}</h2><span>${t.tag}</span><a href="#only=${sys}">#only=${sys}</a><a href="#only=${sys}&theme=${alt}" style="margin-left:14px">#only=${sys}&amp;theme=${alt}</a></div>
   ${headCard(sys, th)}
   ${meta('identity', 'Mark and wordmark', 'drawn SVG marks; the wordmark is live text in the fonts on this machine')}
   <div class="card two" data-part="identity">${identityPane(sys, a)}${identityPane(sys, b)}</div>
   ${meta('colour', 'Colour', 'all ΔE figures are OKLab × 100, computed by tools/palette.mjs (Machado 2009 simulation, severity 1.0)')}
   <div class="card two" data-part="colour">${colourPane(sys, a)}${colourPane(sys, b)}</div>
   ${meta('type', 'Type', `shown in the ${th} theme`)}
   ${typeBlock(sys, th)}
   ${meta('components', 'Components', `shown in the ${th} theme`)}
   ${compsBlock(sys, th)}
   ${meta('context', 'In context', `shown in the ${th} theme — the place to judge the system`)}
   <div class="card eg ctx" data-part="context" data-sys="${sys}" data-theme="${th}">${hero(sys)}${termAndNotebook(sys)}${viewer(sys)}</div>
  </section>`;
}

// ---------- page ----------
(function main() {
  const only = OPT.only && IDS.includes(OPT.only) ? OPT.only : null;
  const page = document.getElementById('page');
  let html = `<header class="intro ov-intro"><h1>Everygraph — visual identity, round 2: five systems descended from “B's system + A's mark”</h1>
    <p>Design mockup; nothing here is final and every figure about a graph is invented. Marks are drawn SVG; wordmarks and all text use fonts installed on the machine showing this page, so they differ slightly elsewhere.
    Show one system with <code>#only=1</code> … <code>#only=5</code>, force a theme with <code>&amp;theme=light|dark</code>, and one block with <code>&amp;part=identity|colour|type|components|context</code>.</p></header>`;
  html += `<div class="ov">${IDS.map(overviewCard).join('')}</div>`;
  for (const id of IDS) if (!only || only === id) html += section(id, OPT.theme === 'dark' || OPT.theme === 'light' ? OPT.theme : SYS[id].primary);
  page.innerHTML = html;
  if (only) { document.body.classList.add('only'); document.querySelector('#s' + only).classList.add('show'); document.querySelector('.ov-intro').style.display = 'none'; }
  if (OPT.part) { document.body.classList.add('part'); const keep = OPT.part.split(','); document.querySelectorAll('[data-part]').forEach(e => { if (keep.includes(e.dataset.part)) e.classList.add('keep'); }); }
  fitWordmarks();
  document.querySelectorAll('[data-face]').forEach(e => { const r = renderedFont(e, e.dataset.face); e.querySelector('.seen').innerHTML = r.first ? 'installed here, so this is the real thing' : `not installed here: you are looking at the fallback, <b>${r.name}</b>`; });
})();
