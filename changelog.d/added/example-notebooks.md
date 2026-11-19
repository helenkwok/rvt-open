- **Three example notebooks, run by CI.** `examples/notebooks` holds
  marimo notebooks that use the public API on the Revit 2024 basic sample
  project: open a model (file information, counts by category, a filterable
  element table, the parameters of a picked element), the parameters across
  a model (the SQLite element table and one chart) and a plan view and a
  sheet as SVG. A CI job builds the Python wheel, runs each as a script
  (a failed cell or assertion fails the job) and uploads each exported as
  static HTML.
