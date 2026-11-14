- **A Revit 2017 to 2022 file's storeys get the GlobalIds Revit's export gives
  them, from the project's own GUID, and a history with an entry closed by
  another byte than `0x28` is read.** Revit's exporter makes a storey's
  GlobalId before Revit 2023 from the project's own GUID (which the header of
  `Global/History` holds four times, at `0x16`, `0x26`, `0x36` and `0x56`) with
  the Level's ElementId XORed into its last 32 bits: 69 of 69 storeys on 11
  files of 2017 to 2022. From Revit 2023 each Level's own episode gives them, as
  before; on 2015 to 2022 an episode's is no longer written for a storey. The
  project, building and site keep the GlobalIds rvt-open already derived from
  the same GUID. `EpisodeHistory::parse` read none of the elements of a
  Revit 2018 model whose history closes four of its 1,127 entries with `0x1e`:
  it reads them now.
