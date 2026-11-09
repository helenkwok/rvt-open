- **A release's `SHA256SUMS` has a build provenance attestation of its own**,
  so verifying it and then checking the sums covers every file on the release
  page; and the release workflow can be run by hand on a branch as a dry run
  that builds, packages and scans every archive and publishes nothing.
