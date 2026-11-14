- **A door's or window's type takes no guessed flip in its GlobalId.**
  Revit hashes a door's type GlobalId with the door's flip, and
  A later reading measures that a door with no host is not flipped. rvt-open also took a
  hosted door whose host frame or facing was not read as not flipped. Such a
  door's type now gets the GlobalId the writer derives, as any type rvt-open
  cannot match does.
