import { deltaE, contrast, separation, oklch, simHex } from '/home/tux/projects/everygraph/design/preview/brand2/tools/color.mjs';
export const TOK = {
  light: { pal: '#3346c7,#c2571a,#1f8a70,#a23b72,#7a6a00,#5f43b2,#b8323c,#2f7fb8,#4d7a1f,#6b6258'.split(','), bg: '#f6f3ec', surface: '#fdfbf7', map: '#f1ede4', accent: '#3346c7',
    seq: '#eef0fb,#cfd4f3,#a9b1ea,#7f8ade,#5665d2,#3346c7,#24328f'.split(','), div: '#b8323c,#dc8a7f,#f1d3cb,#ece8dd,#c7cdf0,#7f8ade,#3346c7'.split(',') },
  dark: { pal: '#93a0ff,#f0975a,#4fc9a8,#e07db0,#d6c545,#b79bff,#f47a80,#6cbcf0,#9fd166,#b3aa9c'.split(','), bg: '#1b1a17', surface: '#24221f', map: '#131210', accent: '#93a0ff',
    seq: '#23243a,#2f3260,#414690,#5a63bd,#7884e3,#93a0ff,#c5ccff'.split(','), div: '#f47a80,#b5606a,#5a3a3f,#2e2c28,#3b3f6e,#6b76cf,#93a0ff'.split(',') } };
export function report(name, pal, t, verbose = true) {
  const s = separation(pal), f = x => x[0].toFixed(1) + ' (' + (pal.indexOf(x[1]) + 1) + '-' + (pal.indexOf(x[2]) + 1) + ')';
  console.log(`\n== ${name}: ${pal.join(',')}`);
  console.log(`  worst pair: normal ${f(s.normal)} · protan ${f(s.protan)} · deutan ${f(s.deutan)} · tritan ${f(s.tritan)}`);
  let below = { normal: 0, protan: 0, deutan: 0, tritan: 0 }, n8 = 0;
  for (let i = 0; i < pal.length; i++) for (let j = i + 1; j < pal.length; j++) { if (deltaE(pal[i], pal[j]) < 15) below.normal++; for (const k of ['protan', 'deutan', 'tritan']) if (deltaE(pal[i], pal[j], k) < 8) below[k]++; }
  console.log(`  pairs below floor (of ${pal.length * (pal.length - 1) / 2}): normal<15 ${below.normal} · protan<8 ${below.protan} · deutan<8 ${below.deutan} · tritan<8 ${below.tritan}`);
  if (verbose) {
    console.log('  contrast  bg / surface / map · L C H · dE to accent');
    pal.forEach((c, i) => { const [L, C, H] = oklch(c); console.log(`   d${i + 1} ${c}  ${contrast(c, t.bg).toFixed(2)} / ${contrast(c, t.surface).toFixed(2)} / ${contrast(c, t.map).toFixed(2)} · ${L.toFixed(3)} ${C.toFixed(3)} ${H.toFixed(0)} · ${deltaE(c, t.accent).toFixed(1)}`); });
    for (const k of [null, 'protan', 'deutan', 'tritan']) { console.log('  matrix ' + (k || 'normal')); pal.forEach((a, i) => console.log('   ' + pal.map((b, j) => i === j ? '  — ' : deltaE(a, b, k).toFixed(0).padStart(4)).join(''))); }
  }
  return s;
}
if (process.argv[1].endsWith('measure.mjs')) {
  for (const th of ['light', 'dark']) { report('token ' + th, TOK[th].pal, TOK[th]);
    for (const r of ['seq', 'div']) { console.log(`  ${r}: ` + TOK[th][r].map(c => oklch(c)[0].toFixed(3)).join(' ') + '  dL: ' + TOK[th][r].slice(1).map((c, i) => (oklch(c)[0] - oklch(TOK[th][r][i])[0]).toFixed(3)).join(' ') + '  C: ' + TOK[th][r].map(c => oklch(c)[1].toFixed(3)).join(' ') + '  H: ' + TOK[th][r].map(c => oklch(c)[2].toFixed(0)).join(' ')); }
  }
}
