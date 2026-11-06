- **Large exports hold their bytes fewer times.** The native passes strip a
  partition's checksum pages where the stream was read instead of in a copy,
  a continuation group made of whole consecutive members is borrowed from the
  inflated stream instead of copied, and the partition's string records are
  scanned chunk by chunk where they lie instead of in a joined copy of the
  whole partition. Exports do not change.
