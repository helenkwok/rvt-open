- **A stair landing of a monolithic type gets its family on Revit 2025 to
  2027, as on Revit 2024.** A landing's family was named only where its landing
  type is not monolithic, so a monolithic landing read from a 2025 to 2027
  save had no family, and with no family no `ObjectType` and no `IfcSlabType`.
  Its landing type's `m_hasMonolithicSupport` now names the family `Monolithic
  Landing`. The family words of a stair's parts are the ones an English Revit
  writes: the file does not hold them, and a Revit in another language writes
  its own, so on such a model the type object is there and its family word
  differs from that export's.
