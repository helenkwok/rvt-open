- **Peak memory back down after the faster exports.** The faster exports
  raised the peak on some models (see "Faster exports, same output").
  Schema objects are now read at most a mebibyte of payload at a time
  besides at most 4,096 objects, and the quantities pass groups mesh edges
  in sorted lists instead of maps. Outputs are byte-identical. Two runs of
  the Memory workflow, before to after, MiB (`rvt-ifc`; `rvt-gltf`): BIM4LCA
  concrete housing 3,590 to 1,630 and 1,631; 3,582 and 3,583 to 1,594 and
  1,621. BIM4LCA timber housing 3,522 to 1,632 and 1,643; 3,515 and 3,516
  to 1,602 and 1,568. BIM4LCA concrete office 2,503 and 2,502 to 1,960 and
  1,985; 2,502 and 2,500 to 1,583 and 1,599. `rac_basic` 2024 1,459 and
  1,426 to 1,227 and 1,256; 1,244 and 1,159 to 1,166 and 1,159. `rst_basic`
  2024 382 to 331 and 340; 388 and 435 to 329 and 328. Snowdon 2024 2,543
  and 2,527 to 2,530 and 2,486; 2,114 and 2,190 to 2,097 and 2,063. Times
  are equal or a little lower. Before the faster exports the housing
  models peaked at about 2,400 to 2,500 MiB and `rac_basic` 2024 at about
  1,210 MiB.
