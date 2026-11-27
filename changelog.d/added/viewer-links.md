- **The viewer lists a model's links and places a linked model the user gives.** A Links
  list beside the scene tree shows the Manage Links rows of the open file (kind, name,
  status, Attachment or Overlay, saved path and path type, placements) with each link's
  placements under it. A link whose file is not given stays listed as saved, with its
  saved path; nothing is fetched. Giving the linked `.rvt` (the row's Open file… button,
  or dropping a file with the link's name while the model is open) draws it and places
  one copy per placement with the transform the host file stores. New wasm binding
  `linksJson`.
