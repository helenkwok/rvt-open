- **A stair run is called spiral only where its treads lie about one centre.**
  A flight was written as `SPIRAL`, and its stair as `SPIRAL_STAIR`, wherever
  every tread of the run gives `m_runType` 2. Revit's export of a Revit 2027
  model with no spiral run does not call three such runs spiral (their flights'
  types are `USERDEFINED` with an `ElementType` of `NOTDEFINED`). Such a run is
  now a spiral only where its treads pass the spiral test the 2024 spiral
  treads are drawn with (one centre, one pair of radii, each tread beside and
  above the one before), and its flight and stair keep `NOTDEFINED` otherwise.
  On the public samples nothing changes: the spiral flights and stairs of the
  BIM4LCA office and of Snowdon Towers Architectural keep their types.
