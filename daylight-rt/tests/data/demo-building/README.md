# The demo building's daylight reference

Derived from a 40 x 30 ft single-storey demo building its publisher released under CC0 1.0 (public domain); the model itself is the release asset `fixtures-cc0-1` (`tools/ci/fetch-cc0-models.sh`), whose `SOURCE.md` says where it comes from. Everything here is CC0 like its source.

- `materials.rad`, `scene.rad`: the building as a Radiance scene, built from rvt-open's glTF export (1,968 triangles, 64 plastic and 36 glass modifiers from the glTF materials). `daylight_rt::glb::read` builds the same scene and grid from the `.rvt`; `tests/demo_building.rs` checks it.
- `grid.pts`: 391 sensors, 0.76 m above the floor tops, 0.5 m apart, kept 0.35 m in from the floor's edge (the file has no rooms).
- `sun_dirs.txt`: month, day, hour and the direction `gensky M D H -a 42.35866 -o 71.05674 -m 75 +s` gives, for 20 March, 21 June and 21 December, 05:00 to 19:00, above the horizon (36 positions).
- `ref_sun.dat`: per sensor, how many of those suns reach it, `rtrace -I -h -ab 0 -dj 0` against a `light` source of 0.533°.
- `ref_svf.dat`: sky view factor, `rtrace -I -h -n 8 -ab 1 -aa 0 -ad 16384 -lw 1e-6` under a uniform `glow` sky, divided by pi.
- `df_p1.dat`, `df_p1_n8.dat`: irradiance (RGB) under `gensky -ang 45 0 -c -B 55.866` with sky and ground `glow`, `rtrace -I -h -ab 2 -aa 0.1 -ad 2048 -ar 64`, one process and eight; DF = 100 x 179 x (0.265 R + 0.670 G + 0.065 B) / 10,000. `df_p2.dat`, `df_p2_n8.dat`: the same at `-ab 5 -ad 2048 -as 1024 -ar 256 -aa 0.15 -lr 6 -lw 2e-4`.

Radiance 6.0.3 (the Linux release binaries), run on the scene and sensors above.
