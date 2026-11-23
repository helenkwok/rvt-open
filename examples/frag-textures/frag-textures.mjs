// Puts the textures of an rvt-open `.frag` texture sidecar back on one model
// loaded with @thatopen/fragments (tested on 3.6.1). Schema: docs/frag-textures.md.
import * as THREE from "three";

export const SIDECAR_FORMAT = "rvt-open/frag-textures";
export const SIDECAR_VERSIONS = [1];

/** Lower-case hex SHA-256 of `bytes`. */
export async function sha256(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Call before `fragments.load(fragBytes, { modelId })`. Only the model with
 * that id is touched: when one of its tiles is drawn in a textured material,
 * that material (that model's entry in the material list) becomes a
 * MeshStandardMaterial drawing the image, and the tile gets a `uv` attribute
 * from the sidecar's per-corner coordinates. Highlight materials are left
 * as they are.
 *
 * @param fragments a FragmentsModels instance
 * @param modelId the id the model will be loaded under
 * @param sidecar the parsed `<name>.frag.textures.json`
 * @param baseUrl the sidecar's URL (image paths are relative to it)
 * @param fragBytes the `.frag` file's bytes: the sidecar must be its own
 * @returns `{ counts, dispose }`. `counts`: `exact` runs of an item's
 *   vertices given their coordinates, `unaligned` runs whose length did not
 *   match their samples', `projected` runs drawn with a box projection.
 *   `dispose()` removes the listeners, puts the original materials back and
 *   frees the textures.
 */
export async function applyFragTextures(fragments, modelId, sidecar, baseUrl, fragBytes) {
  if (sidecar?.format !== SIDECAR_FORMAT || !SIDECAR_VERSIONS.includes(sidecar.version)) {
    throw new Error(`not a ${SIDECAR_FORMAT} sidecar of version ${SIDECAR_VERSIONS.join(", ")}`);
  }
  const hash = await sha256(fragBytes);
  if (hash !== sidecar.fragSha256) {
    throw new Error(`the sidecar belongs to another .frag (${sidecar.fragSha256}, this one is ${hash})`);
  }
  const loader = new THREE.TextureLoader();
  const maps = new Map(); // image path -> texture, shared by its materials
  const materials = new Map(); // .frag material local id -> sidecar entry
  for (const e of sidecar.materials) materials.set(e.material, e);
  const samples = new Map();
  for (const s of sidecar.samples) {
    const key = `${s.meshItem}/${s.material}`;
    if (!samples.has(key)) samples.set(key, []);
    samples.get(key).push(s);
  }
  const counts = { tiles: 0, vertices: 0, exact: 0, unaligned: 0, projected: 0 };
  const swapped = new Map(); // material list key -> [original, textured]
  const list = fragments.models.materials.list;
  const map = (entry) => {
    if (!maps.has(entry.image)) {
      const t = loader.load(new URL(entry.image, baseUrl).href);
      t.colorSpace = THREE.SRGBColorSpace;
      t.wrapS = t.wrapT = THREE.RepeatWrapping;
      maps.set(entry.image, t);
    }
    return maps.get(entry.image);
  };
  // this model's material for a tile: swap it in the list once.
  const textured = (original, entry) => {
    for (const [, pair] of swapped) if (pair[0] === original || pair[1] === original) return pair[1];
    let key;
    for (const [k, v] of list) if (v === original) { key = k; break; }
    const opacity = entry.color[3] / 255;
    const m = new THREE.MeshStandardMaterial({
      map: map(entry),
      color: 0xffffff, // the image is the colour; the opacity stays the material's
      opacity,
      transparent: opacity < 1 || entry.imageHasAlpha,
      side: original.side,
      roughness: 0.9,
      metalness: 0,
      polygonOffset: original.polygonOffset,
      polygonOffsetFactor: original.polygonOffsetFactor,
      polygonOffsetUnits: original.polygonOffsetUnits,
    });
    m.userData = { ...original.userData, fragTextured: true };
    swapped.set(key ?? original.uuid, [original, m]);
    if (key !== undefined) list.set(key, m);
    return m;
  };

  const onTile = ({ value: mesh }) => {
    const g = mesh.geometry;
    const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
    const entry = materials.get(mats[0]?.userData?.localId);
    if (!entry || g.isLODGeometry || mats[0].isLodMaterial || !g.attributes.id?.array || !g.attributes.position?.array) return;
    const m = textured(mats[0], entry);
    if (Array.isArray(mesh.material)) mesh.material[0] = m;
    else mesh.material = m;
    const ids = g.attributes.id.array;
    const pos = g.attributes.position.array;
    const nor = g.attributes.normal?.array;
    const n = pos.length / 3;
    const uv = new Float32Array(n * 2);
    // the tile's `id` attribute: the item's index in Meshes.meshesItems + 1, big-endian bytes.
    const item = (v) => ((ids[v * 4] << 24) >>> 0) + (ids[v * 4 + 1] << 16) + (ids[v * 4 + 2] << 8) + ids[v * 4 + 3] - 1;
    for (let v = 0; v < n; ) {
      const id = item(v);
      let end = v + 1;
      while (end < n && item(end) === id) end++;
      const list = samples.get(`${id}/${entry.material}`) ?? [];
      const s = list.length === 1 ? list[0] : undefined;
      if (s?.uv && s.uv.length === (end - v) * 2) {
        uv.set(s.uv, v * 2);
        counts.exact++;
      } else {
        const size = list.find((x) => x.projection)?.projection.repeatSizeMetres
          ?? (entry.repeatSizeFeet ?? [3.2808, 3.2808]).map((f) => f * 0.3048);
        boxProject(pos, nor, uv, v, end, size);
        counts[list.some((x) => x.uv) ? "unaligned" : "projected"]++;
      }
      v = end;
    }
    g.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
    counts.tiles++;
    counts.vertices += n;
  };
  const watched = [];
  const onModel = ({ value: model }) => {
    if (model.modelId !== modelId) return;
    model.tiles.onItemSet.add(onTile);
    watched.push(model);
  };
  fragments.models.list.onItemSet.add(onModel);

  const dispose = () => {
    fragments.models.list.onItemSet.remove(onModel);
    for (const model of watched) model.tiles.onItemSet.remove(onTile);
    for (const [key, [original, m]] of swapped) {
      if (list.get(key) === m) list.set(key, original);
      m.dispose();
    }
    for (const t of maps.values()) t.dispose();
    swapped.clear();
    maps.clear();
  };
  return { counts, dispose };
}

// Box projection on the axis plane facing each vertex's normal, one repeat
// per `size` metres: for samples without coordinates.
function boxProject(pos, nor, uv, from, to, size) {
  for (let v = from; v < to; v++) {
    const [x, y, z] = [pos[v * 3], pos[v * 3 + 1], pos[v * 3 + 2]];
    const [ax, ay, az] = nor ? [Math.abs(nor[v * 3]), Math.abs(nor[v * 3 + 1]), Math.abs(nor[v * 3 + 2])] : [0, 1, 0];
    const [u, w] = ax >= ay && ax >= az ? [z, y] : ay >= az ? [x, z] : [x, y];
    uv[v * 2] = u / size[0];
    uv[v * 2 + 1] = w / size[1];
  }
}
