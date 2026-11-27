- **The viewer draws a 3D view the way Revit shows it, from the view's own
  Graphic Display Options in the file.** A new View display panel picks any of
  the file's 3D views (the default one on open) and applies its visual style
  (Wireframe, Hidden Line, Shaded, Shaded with edges, Consistent Colors,
  Textures, Realistic), its Sun, Ambient Light and Shadows sliders, cast and
  ambient shadows (ambient shadows drawn once the camera stops), sketchy lines
  (jitter and extension), depth cueing (read, and off in 3D views as in
  Revit), background (none, sky, gradient; an image is only a path in the file
  and is said so) and realistic exposure. Each value says whether it came from
  the view, is Revit's default because it was not read, or was set by you; any
  of them can be changed, and "Use the view's settings" puts them back. The
  view's sun setting and cast shadows drive the Daylight panel, and its shadow
  switch drives the view's. The lighting scheme is not read yet.
