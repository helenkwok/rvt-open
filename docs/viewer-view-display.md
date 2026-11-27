# View display in the viewer

The viewer's **View display** panel (right-hand side, above Daylight) draws the 3D view the way the file's 3D view is shown in Revit: its Graphic Display Options, read by `rvt::sun_settings` (the model's `sun`, also in the IFCX as `rvtopen::sun`), are applied to the 3D scene when the model opens.

- **3D view**: the views a user can pick are the file's 3D views that are neither templates nor Revit's internal ones, by name; the panel starts on the default 3D view (`{3D}` where the project has one). The view's camera is not read: picking a view changes how the scene is drawn, not where the camera is.
- **Each value says where it came from**: from the view, "not read: Revit's default", or "set by you". Every value can be changed; **Use the view's settings** puts them all back.
- **Daylight**: the view's sun setting gives the Daylight panel its date and time (its own setting, else the project's first preset), and the view's Cast Shadows turn the Daylight panel's shadows on or off. Turning shadows on or off in either panel changes both.
- `?view-display=off` draws as the viewer did before the panel (the baseline of the measurements below).

## What is applied, and how

| Option | Applied as |
|---|---|
| Visual style | Wireframe: edges only, faces not drawn (still picked). Hidden Line: white unlit faces under the edges. Shaded: lit material colours, a textured material shown in its image's average colour; Shaded with edges adds the edges. Consistent Colors: the same colour unlit on every face, with edges. Textures: the material appearance images, unlit. Realistic (and Realistic with edges): lit appearance images, with exposure. A Rendering view draws as Realistic. The edges are the export's feature edges. |
| Sun, Ambient Light (0-100) | The scene's directional and sky lights, scaled so the values most stored in the measured files' 3D views (Sun 30, Ambient Light 0) give the viewer's previous lighting: sun light × (0.5 + Sun/60), sky light × (1 + Ambient/100). Unlit styles ignore them, as Consistent Colors does in Revit. |
| Cast Shadows, Shadows (0-100) | The Daylight panel's sun and shadows; the Shadows slider sets how dark a shadow is (0 none, 100 black where only sky light reaches). Shadows fall on the model in the lit styles, and on a ground plane under the model in every style. |
| Ambient Shadows | Ambient occlusion (three.js GTAO), drawn once the camera stops and left out while it moves, so navigating is not slowed. Not in Wireframe. |
| Sketchy Lines: Jitter, Extension (0-10) | Each edge extended past its ends by Extension and, with Jitter, drawn as two wavering strokes; short edges jitter and extend less, as in Revit. The same strokes every frame. |
| Depth Cueing: start, end, fade limit | Revit draws depth cueing in elevations and sections only, so in a 3D view it is read and shown but off; ticked, it fades the model toward the background over the model's depth from the camera, reaching the fade limit at the fade end. |
| Background | None: the viewer's canvas colour. Sky: a gradient from the sky to the horizon colour. Gradient: sky, horizon and ground colours. Image: the file keeps only the image's path, so no image is drawn and the panel says so. The background is fixed to the screen; Revit's sky follows the camera's tilt. |
| Realistic exposure | Realistic styles only: filmic tone mapping, exposure 1 when Automatic; a manual value in stops about Revit's default 14, two values a stop (12 is twice as bright). |

**Not applied**: the lighting scheme (exterior or interior, sun only or with artificial lights) is stored as a reference to an element the reader does not decode yet, so it shows "not read" and the scene is lit as exterior, sun only; artificial lights; colour correction (highlights, shadows, saturation, white point); transparency and silhouette line styles; a view template's options where the view uses one (the view's own stored options are drawn).

## Where a value was not read

Revit 2010-2014 files (not measured), and any view whose display objects were not found, give no value for some options; the panel says "not read" and draws Revit's default:

| Option | Default |
|---|---|
| Visual style | none is documented (a view template sets it): Shaded with edges, the viewer's own look |
| Sun, Ambient Light, Shadows | 30, 0, 50 (the values most stored in the measured files' 3D views; the help gives none) |
| Cast Shadows, Ambient Shadows | off |
| Sketchy Lines | off, Jitter 0, Extension 0 (the help: turning sketchy lines on sets both to 0) |
| Depth Cueing | off, start 0 %, end 100 %, fade limit 0 % |
| Background | None (sky, horizon and ground colours as most stored) |
| Exposure | Automatic (the help: the default for new views), value 14 |

A file with no 3D view (the CC0 demo building has templates only) draws every default.

## Reading the stored values (deductions, no answer key)

- The background type is a set of bits: the measured files store 256, 257 and 512. The low byte is taken as the kind in the dialog's order (None, Sky, Gradient, Image): the views storing 257 are the sample projects' elevations and perspective views Revit draws with a sky.
- Every measured view stores exposure type 1 with a value of 14 or 15; the reader names type 1 Manual. Since Revit's help gives Automatic as the default for new views, type 1 may be Automatic; the viewer uses the reader's name, and at 14 the two draw the same.

## Frame times

Measured with `RVT_VIEWER_DISPLAY_MEASURE=1 pnpm exec playwright test tests/view-display-measure.spec.ts` (tables in `measurements/view-display.md`): each case orbits the camera with a 90-step drag and records the interval between animation frames and the time spent issuing each frame, then the one full frame once the camera stops.

The tests are `viewer/tests/view-display.spec.ts`.
