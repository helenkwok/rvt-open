- **Release assets you can verify.** Each CLI archive and the
  WebAssembly tarball now ships with a build provenance attestation
  (`gh attestation verify <file> -R helenkwok/rvt-open`), a CycloneDX
  SBOM and a Trivy report on that SBOM, next to `SHA256SUMS`. Every
  archive and its unpacked contents are scanned with ClamAV, and the
  release is not published if anything is flagged; when configured,
  each archive is also uploaded to VirusTotal, MetaDefender Cloud and
  Hybrid Analysis. The results are in the `scans.md` release asset,
  except Hybrid Analysis's, which are in the release workflow's log.
  SECURITY.md explains each check.
- **Repository scanning.** CodeQL (Rust, TypeScript and the workflows),
  cargo-deny, a Trivy filesystem scan and the OpenSSF Scorecard run on
  pull requests, on `main` and weekly, with results in the Security
  tab; CI also scans the Linux build with ClamAV. Dependabot proposes
  weekly grouped updates for Cargo, the viewer's npm packages and the
  pinned actions. Vulnerabilities are reported through GitHub's
  private vulnerability reporting.
