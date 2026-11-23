# Textures beside a `.frag`

ThatOpen Fragments 3.6 stores a material as one RGBA colour: it has no texture coordinates and no image. A model with textured materials therefore gets a second file beside the `.frag`, the texture sidecar, that holds what the format cannot:

- `rvt-ifc --frag out.frag` writes `out.frag.textures.json`, and the images under `textures/` next to it, when any sample is drawn in a textured material. A model without textures gets no sidecar, and a sidecar an earlier run left beside the `.frag` is removed, with the images it named that the new run does not write (only names of this writer's form are touched).
- In Rust: `rvt::ifc::frag_writer::write_frag_graph_textured` returns the `.frag`, its counts, and the sidecar (`FragTextures`, written with `write_json` as it is made) with its images; `rvt::cli::write_frag_textures` writes them as `rvt-ifc` does. The `.frag` is the one `write_frag_graph_with` writes, so a reader that ignores the sidecar still draws each textured material's average colour.
- In the browser: `convert_all` and `convert_to_handles` give the sidecar as `frag_textures` and each image as `frag_texture:<path>`.
- The images are the same files, by the same names (`textures/<hash>.<ext>`), as the IFCX texture layer writes, so both can share one folder.

## What changes in the `.frag`

Samples of one colour share one `.frag` material, as before; a model without textures writes the same `.frag` as before. For textured materials:

- A textured material is never merged with another material of the same colour: each gets a `.frag` material of its own, so a viewer can put the image back on exactly the samples drawn in it.
- An item's bodies drawn in the same textured material become one shell (one sample), so that each sample is one run of vertices in the loader's tiles.
- A textured shell of more than 21,666 triangles is cut into pieces below the public loader's 65,535-vertex buffer (past which it starts a new run of vertices). Each piece is a sample with a `.frag` material of its own (`part` 1, 2, ...), drawing the same image.

## Sidecar schema, version 1

```json
{
  "format": "rvt-open/frag-textures",
  "version": 1,
  "fragmentsSchema": "3.6.0",
  "model": "<the .frag's model id>",
  "fragSha256": "<SHA-256 of the .frag file>",
  "materials": [ { ... } ],
  "samples": [ { ... } ]
}
```

A reader checks `format`, refuses a `version` it does not know, and checks that `fragSha256` is the SHA-256 (lower-case hex) of the `.frag` file it loads. `model` is the `.frag`'s own model id (its `guid` field, the IFCX header id), for display.

Each entry of `materials` is one textured `.frag` material:

| Key | Value |
|---|---|
| `material` | the material's local id in the `.frag` (`Meshes.materialIds`) |
| `index` | its index in `Meshes.materials` |
| `part` | 0, or the piece of a cut shell this material draws (see above) |
| `name` | the source material's name |
| `image` | the image's path, relative to the sidecar |
| `mimeType` | `image/png` or `image/jpeg` |
| `imageName` | the image's file name in the source, where it names one |
| `imageHasAlpha` | whether the image can hold transparency |
| `color` | the `.frag` material's `[r, g, b, a]` (0 to 255): the average colour readers without the sidecar draw; `a` is the material's opacity |
| `doubleSided` | as the `.frag` material (translucent materials are two-sided) |
| `repeatSizeFeet` | the width and height one repeat of the image covers on a face, as the source gives it |
| `mapping` | the source's own record of how the image is placed (scale, offset, rotation, tint...), as the reader read it, with absolute paths cut to their file names. No reader fills it yet. |

Each entry of `samples` is one `.frag` sample drawn in a textured material:

| Key | Value |
|---|---|
| `item` | the item's local id |
| `meshItem` | the item's index in `Meshes.meshesItems` (the value a sample's `item` field holds) |
| `sample` | the sample's index in `Meshes.samples` |
| `material` | the textured material's local id |
| `triangles` | the number of triangles of the sample's shell |
| `uv` | one `u, v` pair per triangle corner, flattened, in the order the shell's profiles list the corners; `v` points up (OpenUSD's `st`, three.js's default); single precision, as the `.frag`'s points. A corner whose coordinate is missing or not a number is written as `0, 0` and counted (`textured_corners_defaulted`) |
| `projection` | instead of `uv`, where the sample has no texture coordinates (a pipe written as a circle extrusion, a body without coordinates): `{"kind": "box", "repeatSizeMetres": [w, h]}`, for a box projection |

The writer's counts (`--frag-stats`) say what it did: `textured_bodies_merged`, `textured_shells_split`, `textured_duplicate_keys` (samples with coordinates sharing an item and a material; 0 by construction), `textured_corners_defaulted` and `textured_samples_skipped` (samples whose material had no image to list).

Texture coordinates are given per corner, not per point, because the public loader lays out a shell's vertices three per triangle, in profile order: corner `k` of triangle `t` of a sample is vertex `3t + k` of that sample's run in a tile.

## Viewer

[`examples/frag-textures/`](../examples/frag-textures/) holds `frag-textures.mjs`, a loader for `@thatopen/fragments` (tested on 3.6.1). `await applyFragTextures(fragments, modelId, sidecar, sidecarUrl, fragBytes)`, called before `fragments.load(fragBytes, { modelId })`, checks the sidecar's `fragSha256` against the bytes and then, for that model only:

1. when a tile is drawn in a textured material (its `userData.localId` names it), replaces that model's entry in the material list with a `MeshStandardMaterial` drawing the image; highlight materials are left as they are;
2. gives the tile a `uv` attribute, through `model.tiles.onItemSet`: the tile's `id` attribute names the item of each vertex, and each run of one item's vertices takes its sample's corner coordinates.

It returns `{ counts, dispose }`: counts of what it matched (runs given exact coordinates, runs whose length did not match, runs drawn with a box projection), so a page can check the alignment, and `dispose()`, which removes its listeners, puts the original materials back and frees the textures. CI draws the synthetic models of `tests/material_textures.rs` with it and requires every sample with coordinates to match exactly. The demo page draws a `.frag` with and without its sidecar, or a `.glb` of the same model, from the same camera:

```sh
cd examples/frag-textures && npm ci && npx vite
# http://localhost:5173/?frag=/model.frag            (the sidecar is <frag>.textures.json)
# http://localhost:5173/?frag=/model.frag&textures=0 (colours only)
# http://localhost:5173/?glb=/model.glb
```

Files to view go in `examples/frag-textures/public/`.
