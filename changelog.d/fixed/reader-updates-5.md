- **Revit 2015 files follow the export phase,** as Revit 2016 to 2023 files
  do.
- **A plumbing fixture whose type draws nothing visible is left out**, as
  Revit's export leaves it out; the diagnostics count what is left out.
- **An IFC export holding a number that is not finite is refused** instead of
  being written as `NaN` or `inf`, which no IFC reader opens.
- **A roof drawn as its record box no longer cuts the top of the walls under
  it**, unless the box is exactly the roof's layers and its outline is read.
- **A type's value block ends where the next native record starts**, so the
  two doors of `rac_basic`'s 2027 save keep their Width and Height.
- **A door set in no wall is placed at its stored origin**, which Revit's
  export gives its placement.
- **The sloped line of a wall's edited elevation profile goes on, along its
  slope, to the wall's end.**
