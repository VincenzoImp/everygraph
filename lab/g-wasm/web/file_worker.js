// Read a user File into memory two ways: File.stream() and Blob.slice().arrayBuffer().
onmessage = async (ev) => {
  const f = ev.data.file, r = { size: f.size };
  let t = performance.now(), n = 0;
  const rd = f.stream().getReader();
  for (;;) { const { done, value } = await rd.read(); if (done) break; n += value.byteLength; }
  r.stream_MBps = n / 1048576 / ((performance.now() - t) / 1000);
  t = performance.now(); n = 0;
  for (let off = 0; off < f.size; off += 16 << 20) n += (await f.slice(off, off + (16 << 20)).arrayBuffer()).byteLength;
  r.slice16M_MBps = n / 1048576 / ((performance.now() - t) / 1000);
  // FileReaderSync (worker-only, synchronous)
  t = performance.now(); n = 0;
  const frs = new FileReaderSync();
  for (let off = 0; off < f.size; off += 16 << 20) n += frs.readAsArrayBuffer(f.slice(off, off + (16 << 20))).byteLength;
  r.readersync16M_MBps = n / 1048576 / ((performance.now() - t) / 1000);
  postMessage(r);
};
