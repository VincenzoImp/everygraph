import json, os, resource, time, statistics, sys, subprocess
ROOT = "/home/tux/graphlab/exp/f2-python"
DATA = f"{ROOT}/data"
RES = f"{ROOT}/results"
QUICK = os.environ.get("QUICK", "0") == "1"

def queries(name="lj"):
    q = json.load(open(f"{RES}/queries_{name}.json"))
    if QUICK:
        q["nodes"] = q["nodes"][:50]; q["nodes3"] = q["nodes3"][:15]; q["pairs"] = q["pairs"][:15]
    return q

def truth(name="lj"):
    p = f"{RES}/truth_{name}.json"
    return json.load(open(p)) if os.path.exists(p) else None

def peak_rss_mb():
    return resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024

def du_mb(path):
    if not os.path.exists(path): return None
    out = subprocess.run(["du", "-sb", path], capture_output=True, text=True).stdout.split()[0]
    return int(out) / 1e6

def bench(fn, inputs, timeout_s=10.0, warm=3):
    """Run fn(x) for each input; returns stats dict + list of results. fn may raise TimeoutError."""
    for x in inputs[:warm]:
        try: fn(x)
        except Exception: pass
    lat, outs, tmo, err = [], [], 0, 0
    for x in inputs:
        t = time.perf_counter()
        try:
            r = fn(x)
        except TimeoutError:
            tmo += 1; outs.append("timeout"); lat.append(timeout_s); continue
        except Exception as e:
            err += 1; outs.append(f"error: {str(e)[:200]}"); continue
        dt = time.perf_counter() - t
        if dt > timeout_s:
            tmo += 1
        lat.append(dt); outs.append(r)
    lat_ms = sorted(l * 1e3 for l in lat)
    st = {"n": len(inputs), "timeouts": tmo, "errors": err}
    if err: st["first_error"] = next(o for o in outs if isinstance(o, str) and o.startswith("error"))
    if lat_ms:
        st["median_ms"] = statistics.median(lat_ms)
        st["p90_ms"] = lat_ms[min(len(lat_ms) - 1, int(0.9 * len(lat_ms)))]
    return st, outs

def check(outs, ref):
    if ref is None: return None
    bad = sum(1 for a, b in zip(outs, ref) if not isinstance(a, str) and a != b)
    return {"checked": sum(1 for a in outs if not isinstance(a, str)), "mismatches": bad}

def write(engine, d):
    d["quick"] = QUICK
    d["peak_rss_mb"] = peak_rss_mb()
    os.makedirs(RES, exist_ok=True)
    suffix = "_quick" if QUICK else ""
    json.dump(d, open(f"{RES}/goalA_{engine}{suffix}.json", "w"), indent=1, default=str)
    print(json.dumps(d, indent=1, default=str)[:4000])
