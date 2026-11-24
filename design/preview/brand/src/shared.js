// ---- shared helpers for the brand mockups (inlined by build.py) ----
function egHash(defaults){
  const o=Object.assign({},defaults||{});
  location.hash.replace(/^#/,'').split('&').forEach(kv=>{const[k,v]=kv.split('=');if(k)o[k]=decodeURIComponent(v||'');});
  return o;
}
window.addEventListener('hashchange',()=>location.reload());

// Mark: dir A|B|C, px size. Simplified drawing at <=20 px. Colours come from the
// CSS custom properties of the surrounding .eg scope, so marks follow the theme.
function egMark(dir,size){
  const small=size<=20, a=`width="${size}" height="${size}" viewBox="0 0 32 32" role="img" aria-label="Everygraph mark"`;
  if(dir==='A') return small
    ? `<svg ${a}><circle cx="16" cy="16" r="13.2" fill="none" stroke="var(--brand)" stroke-width="3.6"/><circle cx="13.2" cy="18.6" r="5.6" fill="var(--accent)"/></svg>`
    : `<svg ${a}><circle cx="16" cy="16" r="14" fill="none" stroke="var(--brand)" stroke-width="2"/><circle cx="13" cy="18.6" r="8.4" fill="none" stroke="var(--brand)" stroke-width="2"/><circle cx="11" cy="20.6" r="3.4" fill="var(--accent)"/></svg>`;
  if(dir==='B'){const w=small?4.2:3.2;
    return `<svg ${a}><rect width="32" height="32" fill="var(--ink)"/><path d="M7 7.5 16 25.5 25 7.5" fill="none" stroke="var(--bg)" stroke-width="${w}" stroke-linejoin="miter" stroke-miterlimit="8"/><path d="M10.6 14.8H21.4" stroke="var(--accent)" stroke-width="${w}"/></svg>`;}
  return small
    ? `<svg ${a}><rect width="32" height="32" rx="9" fill="var(--brand)"/><g fill="#fff9f2"><circle cx="5.6" cy="20.8" r="2.2"/><circle cx="12.6" cy="19.2" r="3.8"/><circle cx="22.6" cy="17" r="6"/></g></svg>`
    : `<svg ${a}><circle cx="2.8" cy="21.2" r="2.8" fill="var(--teal,#1f9e8f)"/><circle cx="11.4" cy="19.4" r="4.6" fill="var(--sun,#ffbe3d)"/><circle cx="24.2" cy="16.2" r="7.8" fill="var(--brand)"/></svg>`;
}
// Wordmark as SVG live text (a mockup: the final wordmark would be drawn outlines).
function egWordmarkSVG(dir,h){
  const fs=h*0.82, y=h*0.74;
  let t;
  if(dir==='A') t=`<text x="0" y="${y}" font-size="${fs}" style="font-family:var(--font-display);fill:var(--ink);letter-spacing:-0.01em"><tspan font-style="italic" font-weight="400">every</tspan><tspan font-weight="600" dx="${fs*0.03}">graph</tspan></text>`;
  else if(dir==='B') t=`<text x="0" y="${y}" font-size="${fs*0.86}" style="font-family:var(--font-mono);fill:var(--ink);letter-spacing:-0.04em"><tspan font-weight="400">every</tspan><tspan font-weight="700">graph</tspan></text>`;
  else t=`<text x="0" y="${y}" font-size="${fs}" style="font-family:var(--font-display);fill:var(--ink);letter-spacing:-0.025em" font-weight="800">everygraph</text>`;
  return `<svg class="eg-wm" height="${h}" role="img" aria-label="everygraph" style="overflow:visible">${t}</svg>`;
}
function egLockup(dir,h){
  return `<span class="eg-lockup" style="display:inline-flex;align-items:center;gap:${Math.round(h*0.32)}px;line-height:0">${egMark(dir,Math.round(h*0.92))}${egWordmarkSVG(dir,h)}</span>`;
}
// After insertion: fit every wordmark svg to its text (font metrics differ per machine).
function egFitWordmarks(root){
  (root||document).querySelectorAll('svg.eg-wm').forEach(s=>{
    const b=s.querySelector('text').getBBox(), h=+s.getAttribute('height');
    s.setAttribute('width',Math.ceil(b.width+1)); s.setAttribute('viewBox',`${b.x} 0 ${Math.ceil(b.width+1)} ${h}`);
  });
}
function egFill(root){
  (root||document).querySelectorAll('[data-mark]').forEach(e=>{const d=e.closest('[data-dir]').dataset.dir;e.innerHTML=egMark(d,+e.dataset.mark);});
  (root||document).querySelectorAll('[data-lockup]').forEach(e=>{const d=e.closest('[data-dir]').dataset.dir;e.innerHTML=egLockup(d,+e.dataset.lockup);});
  (root||document).querySelectorAll('[data-wordmark]').forEach(e=>{const d=e.closest('[data-dir]').dataset.dir;e.innerHTML=egWordmarkSVG(d,+e.dataset.wordmark);});
  egFitWordmarks(root);
}
const EG_DIRS={
  A:{name:'Atlas',tag:'cartographic and calm',
     why:'The product is a map, so the brand borrows from atlases: paper, sea and ink, a serif for titles, hairline rules. It says "reference work you can cite" rather than "startup".',
     mark:'Nested, off-centre contours: the picture the map itself draws (a community inside a community), and a contour line.',
     face:'Titles: Charter / Iowan Old Style / Palatino (system serifs). Text: system sans.',
     cvd:{light:'all 45 pairs: worst colour-blind ΔE 7.7 (just under the 8 target), normal vision 15.5',dark:'all 45 pairs: worst colour-blind ΔE 8.0, normal vision 16.1'}},
  B:{name:'Datum',tag:'technical and crisp',
     why:'Black, white and one signal orange; square corners; every number in monospace. It looks like an instrument and promises exactness — the "trust it" half of the third promise.',
     mark:'∀, the "for every" sign of logic, cut into a square: every graph, every size, every machine. The crossbar carries the one accent.',
     face:'Proposed typefaces (both SIL Open Font Licence): Inter for text, JetBrains Mono for numbers and code. Shown here with the closest fonts on this machine.',
     cvd:{light:'all 45 pairs: worst colour-blind ΔE 9.1, normal vision 18.1',dark:'all 45 pairs: worst colour-blind ΔE 9.1, normal vision 18.1'}},
  C:{name:'Marble',tag:'warm and approachable',
     why:'Round shapes, a warm cream page, coral and violet. It tells an analyst or a student that a billion-edge graph is something they are allowed to open.',
     mark:'Three marbles, small to large, on one floor: every size, same tool. No lines between them.',
     face:'Titles and text: the system rounded face where there is one (ui-rounded, SF Rounded, Nunito), otherwise a humanist sans.',
     cvd:{light:'all 45 pairs: worst colour-blind ΔE 8.5, normal vision 16.2',dark:'all 45 pairs: worst colour-blind ΔE 8.8, normal vision 17.4'}}
};
