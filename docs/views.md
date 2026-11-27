# Views, sheets and placed viewports

`rvt::research::views::read` lists a project's views from the file's own
records, and `rvt-views` prints the catalog as JSON. `rvt-sheet` and the
browser viewer already draw plan views, sections and sheets with their
viewports from `plan_views::read`; this catalog adds every other kind of
view, each 3D view's camera and a placed viewport's box centre, as data. It
is a research module, behind the `unstable-research` feature, so it can still
change between releases: the box-centre rule rests on one viewport (below).

## What is read

A view is a `DBView*` record on channel 102. The catalog carries its ElementId,
class, kind (`plan`, `three_d`, `section`, `drafting`, `sheet`, `legend`,
`schedule` or `other`), name, description, scale, view plane (`m_origin`,
`m_horzDir`, `m_vertDir`, `m_viewDir`), view range (plans), template and
whether it is a template. A 3D view also carries its `Viewer`'s camera: eye
(`m_boundedSpace.m_orig`), up and looking direction (`m_basis`), and the
stored target where that target lies on the looking axis. The camera is
accepted only when the basis is orthonormal and agrees with the view plane's
own axes, so a set of fields that does not describe one frame is not reported
as one.

A sheet is a `DBViewDrafting` record with an `m_sheetNumber`: its number, name,
date, drawn-by, checked-by, designed-by, approved-by and issue-date fields,
whether it is scheduled, and the viewport ids its record lists.

A viewport is a `Viewport` record. Every view carries one internally; a
viewport placed on a sheet also carries `m_ownerDBViewId` (the sheet's view)
and `m_dbViewId` (the view it shows). The catalog reports its detail number,
offset, scale, orientation, label origin and length, the view's crop box, the
title settings of its `ViewportAttributes` type, and its box centre where one
is measured:

```text
centre = m_offset + m_scale * ((crop.x_min + crop.x_max) / 2,
                               (crop.y_min + crop.y_max) / 2)
```

`m_offset` is where the view's origin lands on the sheet. One viewport of a
contributor's Revit 2024 model was compared with the Revit API's
`Viewport.GetBoxCenter()` and agreed to the five decimals it printed.
A viewport turned on its sheet (orientation other than 0), a view whose crop
region is off and a viewport whose crop box is not read have no centre here:
the turn and the uncropped extent are not measured.

## Usage

```rust
// Cargo.toml: rvt = { package = "rvt-open", version = "...", features = ["unstable-research"] }
use rvt::RevitFile;
use rvt::research::views::ViewCatalog;

let mut file = RevitFile::open("project.rvt")?;
let catalog = ViewCatalog::read(&mut file)?;
for sheet in &catalog.sheets {
    println!("{} {}", sheet.number, sheet.name);
    for viewport in catalog.viewports_on(sheet.id) {
        println!("  view {} at {:?}", viewport.view_id, viewport.center);
    }
}
```

```bash
cargo install rvt-open --features unstable-research
rvt-views project.rvt
rvt-views large-project.rvt --large-model > views.json
```

## Evidence boundary

- The box-centre rule is measured on **one** viewport of one Revit 2024
  project, against the Revit API. It is not re-measured on other models,
  because no Revit API dump of them is here.
- Against `plan_views::read`, which `rvt-sheet` and the viewer draw sheets
  from, on 53 public models (Autodesk's samples of Revit 2015 to 2027,
  Snowdon Towers and the BIM4LCA files): every placed viewport both read has
  the same sheet, view and offset, every sheet the catalog reads has the same
  number and name, and every plan view both read has the same scale.
  `plan_views::read` places more: on Revit 2015 to 2022 the catalog reads the
  viewport records but not their sheet and view, so it places none (the
  structural samples: 0 against 23 and 30), and on later releases it misses
  placed schedules and other placements that are not `Viewport` records
  (1 to 10 a model). `tests/views.rs` checks the agreement on the BIM4LCA
  office.
- What the catalog adds: every kind of view (plans, sections, 3D views,
  drafting views, legends, schedules, sheets), each 3D view's camera (on
  Revit 2023 to 2027; for example 77 of the DACH sample's 82 3D views and 37
  of Snowdon Towers Architectural's 44) and the box centre of a placed,
  unturned, cropped viewport (104 on the DACH sample, 103 on Snowdon Towers
  Architectural).
- The synthetic `corpus/tier1/` files are writer-generated and hold no native
  partition records, so `read` returns their stream error rather than an empty
  catalog. A test uses the office model under `RVT_NORDIC_LCA_DIR` instead.
- Not read: view graphics and visibility overrides, per-view filters,
  the view title's printed position, a turned viewport's placement, the
  uncropped view's extent, sheet size, and anything that needs Revit to
  print. The browser viewer, Python bindings and MCP server do not expose the
  catalog yet.
