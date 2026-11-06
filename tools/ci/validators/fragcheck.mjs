// .frag check with ThatOpen's own reader (@thatopen/fragments, SingleThreadedFragmentsModel).
import { SingleThreadedFragmentsModel } from "@thatopen/fragments";
import fs from "fs";
const vol = (g) => { let v = 0, any = false; const walk = (o) => { if (!o || typeof o !== "object") return;
  if (o.indices && o.positions && o.indices.length !== undefined) { any = true; const p = o.positions, ix = o.indices;
    for (let i = 0; i + 2 < ix.length; i += 3) { const a = ix[i] * 3, b = ix[i + 1] * 3, c = ix[i + 2] * 3;
      v += (p[a] * (p[b + 1] * p[c + 2] - p[b + 2] * p[c + 1]) - p[a + 1] * (p[b] * p[c + 2] - p[b + 2] * p[c]) + p[a + 2] * (p[b] * p[c + 1] - p[b + 1] * p[c])) / 6; } return; }
  for (const x of Array.isArray(o) ? o : Object.values(o)) walk(x); }; walk(g); return any ? Math.abs(v) : null; };
const tri = (g) => { let t = 0; const walk = (o) => { if (!o || typeof o !== "object") return;
  if (o.indices && o.indices.length !== undefined) { t += o.indices.length / 3; return; }
  for (const v of Array.isArray(o) ? o : Object.values(o)) walk(v); }; walk(g); return t; };
for (const p of process.argv.slice(2)) {
  const out = { file: p.split("/").pop() }; let m;
  try {
    m = new SingleThreadedFragmentsModel("m", new Uint8Array(fs.readFileSync(p)));
    await Promise.race([m.ready, new Promise((_, r) => setTimeout(() => r(new Error("ready timeout 300s")), 300000))]);
    out.loaded = true;
    const geo = await m.getItemsIdsWithGeometry(); out.withGeometry = geo.length;
    out.maxLocalId = await m.getMaxLocalId();
    out.categories = (await m.getCategories()).length;
    const guids = await m.getGuidsByLocalIds(geo); out.geoWithoutGuid = guids.filter((g) => !g).length;
    out.duplicateGuids = guids.length - new Set(guids.filter(Boolean)).size - guids.filter((g) => !g).length;
    const data = await m.getItemsData(geo, { attributesDefault: true, relationsDefault: { attributes: false, relations: false } });
    out.geoWithoutName = data.filter((d) => !(d?.Name?.value)).length;
    const per = [], vols = []; for (let i = 0; i < geo.length; i += 500) { const g = await m.getItemsGeometry(geo.slice(i, i + 500)); for (const x of g) { per.push(tri(x)); const vv = vol(x); vols.push(vv); } }
    try { const bc2 = await m.getItemsOfCategories([/.*/]); const cat = new Map(); for (const [k, v] of Object.entries(bc2)) for (const x of v) cat.set(x, k); const z = {}; vols.forEach((x, i) => { if (x !== null && !(x > 1e-9)) { const k = cat.get(geo[i]) ?? '?'; z[k] = (z[k] || 0) + 1; } }); out.zeroVolumeByCategory = z; } catch {}
    out.zeroVolume = vols.filter((x) => x !== null && !(x > 1e-9)).length; out.volumeUnknown = vols.filter((x) => x === null).length;
    out.triangles = per.reduce((a, b) => a + b, 0); out.zeroTriangleItems = per.filter((x) => x === 0).length;
    out.boxLike12 = per.filter((x) => x === 12).length;
    try { const g = new Set(geo); const bc = await m.getItemsOfCategories([/.*/]); out.byCategory = Object.fromEntries(Object.entries(bc).map(([k, v]) => [k, v.filter((x) => g.has(x)).length]).filter(([, n]) => n > 0)); } catch (e) { out.byCategoryError = String(e).slice(0, 100); }
    const sp = await m.getSpatialStructure(); let n = 0, d = 0; const w = (o, k) => { n++; d = Math.max(d, k); for (const c of o?.children ?? []) w(c, k + 1); }; w(sp, 0);
    out.spatialNodes = n; out.spatialDepth = d;
    try { out.crs = (typeof m.getCRS === 'function' ? await m.getCRS() : null) ?? null; } catch { out.crs = null; }
  } catch (e) { out.error = String(e?.stack ?? e).slice(0, 400); }
  finally { try { m?.dispose(); } catch {} }
  console.log(JSON.stringify(out));
}
process.exit(0);
