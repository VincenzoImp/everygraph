// OPFS sync-access-handle throughput probe (dedicated worker only API).
onmessage = async (ev) => {
  const { totalMB = 1024, chunkMB = 16 } = ev.data || {};
  const r = {};
  try {
    r.estimate_before = await navigator.storage.estimate();
    r.persisted = await navigator.storage.persisted();
    const root = await navigator.storage.getDirectory();
    const fh = await root.getFileHandle('probe.bin', { create: true });
    let h = await fh.createSyncAccessHandle();
    h.truncate(0);
    const buf = new Uint8Array(chunkMB * 1048576);
    for (let i = 0; i < buf.length; i += 4096) buf[i] = i & 255;
    const total = totalMB * 1048576;
    let t = performance.now();
    let wrote = 0;
    for (let off = 0; off < total; off += buf.length) wrote += h.write(buf, { at: off });
    r.bytes_written = wrote;
    h.flush();
    r.write_MBps = totalMB / ((performance.now() - t) / 1000);
    r.size = h.getSize();
    h.close();
    h = await fh.createSyncAccessHandle();
    t = performance.now();
    let got = 0;
    for (let off = 0; off < total; off += buf.length) got += h.read(buf, { at: off });
    r.bytes_read = got;
    r.seq_read_MBps = totalMB / ((performance.now() - t) / 1000);
    // random 64 KiB reads (out-of-core access pattern)
    const small = new Uint8Array(65536);
    const nr = 4096;
    let x = 12345;
    t = performance.now();
    for (let i = 0; i < nr; i++) { x = (x * 1103515245 + 12345) >>> 0; h.read(small, { at: (x % (total / 65536)) * 65536 }); }
    const dt = (performance.now() - t) / 1000;
    r.rand64k_MBps = nr * 64 / 1024 / dt;
    r.rand64k_iops = nr / dt;
    h.close();
    r.estimate_after = await navigator.storage.estimate();
    await root.removeEntry('probe.bin');
    r.ok = true;
  } catch (e) { r.ok = false; r.error = String(e); }
  postMessage(r);
};
