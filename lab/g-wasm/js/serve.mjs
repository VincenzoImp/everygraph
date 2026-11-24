// Static server, 127.0.0.1 ONLY, with COOP/COEP for SharedArrayBuffer (cross-origin isolation).
//   /           -> web/
//   /data/csr/<name>/<file> -> /home/tux/graphlab/data/csr/<name>/<file> (read-only)
//   /data/txt/<file>        -> exp/g-wasm/data/<file>
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', 'web');
const CSR = '/home/tux/graphlab/data/csr';
const TXT = path.resolve(here, '..', 'data');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json' };
export function serve(port = 0) {
  const srv = http.createServer((req, res) => {
    const u = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    let f;
    if (u.startsWith('/data/csr/')) f = path.join(CSR, path.normalize(u.slice(10)));
    else if (u.startsWith('/data/txt/')) f = path.join(TXT, path.normalize(u.slice(10)));
    else f = path.join(root, path.normalize(u === '/' ? 'index.html' : u));
    if (!(f.startsWith(CSR) || f.startsWith(TXT) || f.startsWith(root)) || !fs.existsSync(f) || fs.statSync(f).isDirectory()) { res.writeHead(404); return res.end(); }
    res.writeHead(200, {
      'Content-Type': types[path.extname(f)] || 'application/octet-stream',
      'Content-Length': fs.statSync(f).size,
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp',
      'Cross-Origin-Resource-Policy': 'same-origin',
      'Cache-Control': 'no-store',
    });
    fs.createReadStream(f, { highWaterMark: 4 << 20 }).pipe(res);
  });
  return new Promise((ok) => srv.listen(port, '127.0.0.1', () => ok(srv)));
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const s = await serve(Number(process.argv[2] || 8765));
  console.log('listening', s.address());
}
