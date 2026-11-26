// m1-06 (#20): the preview's keyboard model, checked (design/DECISIONS.md, "The map's keyboard model"):
// arrows move between regions and through the hierarchy, Enter selects, the matrix is a grid, Esc goes back, and
// every move is announced in the live region. Exits 1 on the first failed check.
//   node keys_check.mjs OVERVIEW_URL OUT.json
import fs from 'node:fs';
import { chromium } from 'playwright';

const [url, out] = process.argv.slice(2);
const b = await chromium.launch(); const p = await b.newPage({ viewport: { width: 1440, height: 900 } });
const errors = []; p.on('pageerror', e => errors.push(String(e)));
await p.goto(url); await p.waitForFunction(() => window.__ov); await p.waitForTimeout(300);
const checks = [];
const ok = (name, cond, got) => { checks.push({ name, ok: !!cond, got }); if (!cond) console.error('FAIL', name, got); };
const st = () => p.evaluate(() => ({ ...window.__ov.state, status: document.getElementById('status').textContent }));

await p.focus('#map');
let s = await st(); ok('focus announces the first region', /^Region 1, 1 of \d+/.test(s.status), s.status);
await p.keyboard.press('ArrowRight'); s = await st(); ok('ArrowRight moves to the next region', /^Region 2, 2 of/.test(s.status), s.status);
await p.keyboard.press('ArrowLeft'); await p.keyboard.press('Enter'); s = await st(); ok('Enter selects the focused region', s.sel !== '', s.sel);
await p.keyboard.press('Escape'); s = await st(); ok('Escape clears the selection', s.sel === '', s.sel);
await p.keyboard.press('ArrowDown'); s = await st(); ok('ArrowDown opens the region', s.p !== '0' && /^Inside Region 1/.test(s.status), [s.p, s.status]);
await p.keyboard.press('ArrowUp'); s = await st(); ok('ArrowUp goes back to the parent', s.p === '0', s.p);
const before = await p.evaluate(() => window.__ov.state.g); await p.keyboard.press('Shift+ArrowRight'); s = await st(); ok('Shift+arrow pans without changing the state', s.g === before && s.sel === '', s);
await p.evaluate(() => window.__ov.set({ matrix: '1' })); await p.focus('#matrix .c[tabindex="0"]');
ok('the matrix is a grid with one cell in the tab order', await p.evaluate(() => document.querySelectorAll('#matrix [role=grid]').length === 1 && document.querySelectorAll('#matrix .c[tabindex="0"]').length === 1), null);
await p.keyboard.press('ArrowRight'); await p.keyboard.press('ArrowDown');
const read = await p.textContent('#mread'); ok('moving in the grid reads the cell out', /edges|no edge|itself/.test(read), read);
await p.keyboard.press('ArrowRight'); await p.keyboard.press('Enter'); s = await st(); ok('Enter on a cell draws the pair', /^\d+-\d+$/.test(s.pair), s.pair);
await p.keyboard.press('Escape'); s = await st(); ok('Escape closes the matrix', s.matrix === '0', s.matrix);
const k = () => p.evaluate(() => window.__ov.view.k);
await p.focus('#tbconn'); const k0 = await k(); await p.keyboard.press('+'); const k1 = await k();
await p.focus('#map'); await p.keyboard.press('+'); const k2 = await k();
ok('+ zooms only while the map has focus (WCAG 2.1.4)', k1 === k0 && k2 > k1, [k0, k1, k2]);
ok('no page errors', errors.length === 0, errors);
fs.writeFileSync(out, JSON.stringify({ url, checks, passed: checks.filter(c => c.ok).length, total: checks.length }, null, 1));
console.log(checks.filter(c => c.ok).length + ' of ' + checks.length + ' checks passed');
await b.close(); process.exit(checks.every(c => c.ok) ? 0 : 1);
