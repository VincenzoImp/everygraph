import fs from 'node:fs';
const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
const E = instance.exports;
const txt = fs.readFileSync('/home/tux/graphlab/exp/g-wasm/data/lj10.txt');
const p = E.ga_alloc(txt.length); new Uint8Array(E.memory.buffer, p, txt.length).set(txt);
let t=performance.now(); E.ga_parse_owned(p, txt.length); console.log('parse', (performance.now()-t)|0);
t=performance.now(); E.ga_build_csr(); console.log('csr', (performance.now()-t)|0);
for (let i=0;i<4;i++){ t=performance.now(); E.ga_pagerank(20); console.log('pr', (performance.now()-t)|0); }
for (let i=0;i<3;i++){ t=performance.now(); E.ga_cc(); console.log('cc', (performance.now()-t)|0); }
