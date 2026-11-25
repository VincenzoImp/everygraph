// Shared page shell for the style preview: top navigation, theme switch, hash options.
// Usage: <script src="assets/shell.js" data-page="foundations"></script> at the end of <body>.
// Hash: #theme=light|dark (other keys are left for the page to read through EGStyle.opts).
(function () {
  var pages = [
    ['index', 'Overview'], ['foundations', 'Foundations'], ['components', 'Components'], ['data', 'Data & map'],
    ['viewer', 'Viewer'], ['web', 'Web & docs'], ['terminal', 'Terminal'], ['notebook', 'Notebook'], ['export', 'Figures & export']
  ];
  var me = (document.currentScript && document.currentScript.dataset.page) || 'index';
  var opts = {};
  location.hash.replace(/^#/, '').split('&').forEach(function (kv) { if (kv) { var p = kv.split('='); opts[p[0]] = decodeURIComponent(p[1] || ''); } });
  var theme = opts.theme || 'light';
  document.documentElement.setAttribute('data-theme', theme);
  window.EGStyle = { opts: opts, theme: theme };
  if (opts.chrome === '0') return;   // #chrome=0 hides the shell (for clean screenshots of applied pages)
  var mark = '<svg width="22" height="22" viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="13" fill="none" stroke="currentColor" stroke-width="2.4"/><circle cx="18.5" cy="18" r="7" fill="none" stroke="currentColor" stroke-width="2.4"/><circle cx="20" cy="19.2" r="2.6" fill="var(--accent)"/></svg>';
  var top = document.createElement('div'); top.className = 'shell-top';
  top.innerHTML = '<a class="brand" href="index.html">' + mark + '<span>every<b>graph</b></span></a><nav>' +
    pages.map(function (p) { return '<a href="' + p[0] + '.html"' + (p[0] === me ? ' aria-current="page"' : '') + '>' + p[1] + '</a>'; }).join('') +
    '</nav><button class="btn sm" id="eg-theme" title="Switch theme">' + (theme === 'dark' ? 'Light' : 'Dark') + ' theme</button>';
  document.body.insertBefore(top, document.body.firstChild);
  document.getElementById('eg-theme').onclick = function () {
    opts.theme = theme === 'dark' ? 'light' : 'dark';
    location.hash = Object.keys(opts).map(function (k) { return k + '=' + encodeURIComponent(opts[k]); }).join('&'); location.reload();
  };
  var chip = document.createElement('div'); chip.className = 'mockup-chip'; chip.textContent = 'Style preview · invented data'; document.body.appendChild(chip);
})();
