# Textures beside a `.frag`

ThatOpen Fragments 3.6 stores a material as one RGBA colour: it has no texture coordinates and no image. A model with textured materials therefore gets a second file beside the `.frag`, the texture sidecar, that holds what the format cannot:

- `rvt-ifc --frag out.frag` writes `out.frag.textures.json`, and the images under `textures/` next to it, when any sample is drawn in a textured material. A model without textures gets no sidecar.
- In Rust: `rvt::ifc::frag_writer::write_frag_graph_textured` returns the `.frag`, its counts, and the sidecar with its images. The `.frag` is byte-identical to `write_frag_graph_with`'s, so a reader that ignores the sidecar still draws each textured material's average colour.
- The images are the same files, by the same names (`textures/<hash>.<ext>`), as the IFCX texture layer writes, so both can share one folder.

## What changes in the `.frag`

Samples of one colour share one `.frag` material, as before. A textured material is never merged with another material of the same colour: each textured material gets a `.frag` material of its own, so a viewer can put the image back on exactly the samples drawn in it.

## Sidecar schema, version 1

```json
{
  "format": "rvt-open/frag-textures",
  "version": 1,
  "fragmentsSchema": "3.6.0",
  "model": "<the .frag's guid>",
  "materials": [ { ... } ],
  "samples": [ { ... } ]
}
```

A reader checks `format`, refuses a `version` it does not know, and checks that `model` is the `.frag`'s own guid.

Each entry of `materials` is one textured `.frag` material:

| Key | Value |
|---|---|
| `material` | the material's local id in the `.frag` (`Meshes.materialIds`) |
| `index` | its index in `Meshes.materials` |
| `name` | the source material's name |
| `image` | the image's path, relative to the sidecar |
| `mimeType` | `image/png` or `image/jpeg` |
| `imageName` | the image's file name in the source, where it names one |
| `imageHasAlpha` | whether the image can hold transparency |
| `color` | the `.frag` material's `[r, g, b, a]` (0 to 255): the average colour readers without the sidecar draw; `a` is the material's opacity |
| `doubleSided` | as the `.frag` material (translucent materials are two-sided) |
| `repeatSizeFeet` | the width and height one repeat of the image covers on a face, as the source gives it |
| `mapping` | the source's own record of how the image is placed (scale, offset, rotation, tint...), verbatim, where the reader has one |

Each entry of `samples` is one `.frag` sample drawn in a textured material:

| Key | Value |
|---|---|
| `item` | the item's local id |
| `meshItem` | the item's index in `Meshes.meshesItems` (the value a sample's `item` field holds) |
| `sample` | the sample's index in `Meshes.samples` |
| `material` | the textured material's local id |
| `triangles` | the number of triangles of the sample's shell |
| `uv` | one `u, v` pair per triangle corner, flattened, in the order the shell's profiles list the corners; `v` points up (OpenUSD's `st`, three.js's default) |
| `projection` | instead of `uv`, where the sample has no texture coordinates (a pipe written as a circle extrusion): `{"kind": "box", "repeatSizeMetres": [w, h]}`, for a box projection |

Texture coordinates are given per corner, not per point, because the public loader lays out a shell's vertices three per triangle, in profile order: corner `k` of triangle `t` of a sample is vertex `3t + k` of that sample's run in a tile.

## Viewer

[`examples/frag-textures/`](../examples/frag-textures/) holds `frag-textures.mjs`, a loader for `@thatopen/fragments` that reads the sidecar and:

1. replaces each textured material, through `fragments.models.materials.list.onItemSet`, with a `MeshStandardMaterial` drawing the image (the material's `userData.localId` names it);
2. gives each tile drawn in such a material a `uv` attribute, through `model.tiles.onItemSet`: the tile's `id` attribute names the item of each vertex, and each run of one item's vertices takes its samples' corner coordinates in order.

`applyFragTextures` returns counts of what it matched (runs given exact coordinates, runs whose length did not match and fell back to a box projection), so a page can check the alignment. The demo page draws a `.frag` with and without its sidecar, or a `.glb` of the same model, from the same camera:

```sh
cd examples/frag-textures && npm ci && npx vite
# http://localhost:5173/?frag=/model.frag            (the sidecar is <frag>.textures.json)
# http://localhost:5173/?frag=/model.frag&textures=0 (colours only)
# http://localhost:5173/?glb=/model.glb
```

Files to view go in `examples/frag-textures/public/`.
