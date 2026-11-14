- **A history whose entries are closed by another byte than `0x28` is read.**
  `EpisodeHistory::parse` read none of the elements of a Revit 2018 model whose
  history closes four of its 1,127 entries with `0x1e`: it reads them now, so
  those elements take the GlobalIds Revit derives from their episodes.
