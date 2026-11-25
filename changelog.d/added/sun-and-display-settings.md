- **The file's sun settings and each view's display options are read, and the
  viewer's Daylight panel starts from them.** `rvt::sun_settings` reads every
  `SunAndShadowSettings` element (named presets and each view's own setting:
  type, start and end, interval, azimuth, altitude, ground plane, relative to
  view) and each view's graphic display options (visual style, cast and
  ambient shadows, sun, ambient and shadow intensity, sketchy lines, depth
  cueing, background, exposure). The export model carries them (`sun`), the
  IFCX site gets `rvtopen::sun` (the sun a viewer starts from and the named
  presets), and the `.frag` metadata `sun`. The Daylight panel takes its date,
  time, study range and step from the default 3D view's own sun setting (or the
  project's first preset), turns shadows on when that view casts them, and says
  where each value came from; every field can still be changed.
