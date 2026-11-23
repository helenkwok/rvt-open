// Puts the textures of an rvt-open `.frag` texture sidecar back on a model
// loaded with @thatopen/fragments. Schema: docs/frag-textures.md.
import * as THREE from "three";

export const SIDECAR_FORMAT = "rvt-open/frag-textures";

/**
 * Call before `fragments.load`. Textured materials become
 * MeshStandardMaterial drawing their image; tiles drawn in them get a `uv`
 * attribute from the sidecar's per-corner coordinates.
 *
 * @param fragments a FragmentsModels instance
 * @param sidecar the parsed `<name>.frag.textures.json`
 * @param baseUrl the sidecar's URL (image paths are relative to it)
 * @returns live counts: `exact` runs of an item's vertices given their
 *   coordinates, `ambiguous` runs holding several samples (given in sidecar
 *   order), `unaligned` runs whose length did not match their samples',
 *   `projected` runs drawn with a box projection.
 */
export function applyFragTextures(fragments, sidecar, baseUrl) {
  if (sidecar?.format !== SIDECAR_FORMAT || sidecar.version !== 1) {
    throw new Error(`not a version 1 ${SIDECAR_FORMAT} sidecar`);
  }
  const loader = new THREE.TextureLoader();
  const materials = new Map();
  for (const e of sidecar.materials) {
    const map = loader.load(new URL(e.image, baseUrl).href);
    map.colorSpace = THREE.SRGBColorSpace;
    map.wrapS = map.wrapT = THREE.RepeatWrapping;
    materials.set(e.material, { entry: e, map });
  }
  const samples = new Map();
  for (const s of sidecar.samples) {
    const key = `${s.meshItem}/${s.material}`;
    if (!samples.has(key)) samples.set(key, []);
    samples.get(key).push(s);
  }
  const counts = { tiles: 0, vertices: 0, exact: 0, ambiguous: 0, unaligned: 0, projected: 0 };

  fragments.models.materials.list.onItemSet.add(({ key, value }) => {
    const found = materials.get(value?.userData?.localId);
    if (!found || value.userData.fragTextured || value.isLodMaterial) return;
    const { entry, map } = found;
    const opacity = entry.color[3] / 255;
    const textured = new THREE.MeshStandardMaterial({
      map,
      color: 0xffffff, // the image is the colour; the opacity stays the material's
      opacity,
      transparent: opacity < 1 || entry.imageHasAlpha,
      side: value.side,
      roughness: 0.9,
      metalness: 0,
      polygonOffset: value.polygonOffset,
      polygonOffsetFactor: value.polygonOffsetFactor,
      polygonOffsetUnits: value.polygonOffsetUnits,
    });
    textured.userData = { ...value.userData, fragTextured: true };
    fragments.models.materials.list.set(key, textured);
  });

  fragments.models.list.onItemSet.add(({ value: model }) => {
    model.tiles.onItemSet.add(({ value: mesh }) => {
      const g = mesh.geometry;
      const material = Array.isArray(mesh.material) ? mesh.material[0] : mesh.material;
      const found = materials.get(material?.userData?.localId);
      if (!found || g.isLODGeometry || !g.attributes.id?.array || !g.attributes.position?.array) return;
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
        const list = samples.get(`${id}/${found.entry.material}`) ?? [];
        const corners = list.reduce((a, s) => a + (s.uv ? s.uv.length / 2 : NaN), 0);
        if (list.length > 0 && corners === end - v) {
          let o = v * 2;
          for (const s of list) {
            uv.set(s.uv, o);
            o += s.uv.length;
          }
          counts[list.length > 1 ? "ambiguous" : "exact"]++;
        } else {
          const size = list.find((s) => s.projection)?.projection.repeatSizeMetres
            ?? (found.entry.repeatSizeFeet ?? [3.2808, 3.2808]).map((f) => f * 0.3048);
          boxProject(pos, nor, uv, v, end, size);
          counts[list.some((s) => s.uv) ? "unaligned" : "projected"]++;
        }
        v = end;
      }
      g.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
      counts.tiles++;
      counts.vertices += n;
    });
  });
  return counts;
}

// Box projection on the axis plane facing each vertex's normal, one repeat
// per `size` metres: the fallback where no coordinates line up.
function boxProject(pos, nor, uv, from, to, size) {
  for (let v = from; v < to; v++) {
    const [x, y, z] = [pos[v * 3], pos[v * 3 + 1], pos[v * 3 + 2]];
    const [ax, ay, az] = nor ? [Math.abs(nor[v * 3]), Math.abs(nor[v * 3 + 1]), Math.abs(nor[v * 3 + 2])] : [0, 1, 0];
    const [u, w] = ax >= ay && ax >= az ? [z, y] : ay >= az ? [x, z] : [x, y];
    uv[v * 2] = u / size[0];
    uv[v * 2 + 1] = w / size[1];
  }
}
