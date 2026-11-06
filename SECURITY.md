# Security policy

## Supported versions

rvt-open is pre-1.0 and has one maintainer, so support is kept to what
can actually be delivered:

| Version | Security fixes |
|---|---|
| `main` | yes |
| The latest release line (see Releases) | yes, in its next patch or minor release |
| Older release lines | no, unless a note here says that line is maintained |

No older line is maintained today. A fix lands on `main` first and reaches
users in the next release.

## Reporting a vulnerability

If you believe you've found a security issue — including anything
that would let an attacker cause rvt-open to crash, mis-decode, or
leak data when fed a hostile input file — please **do not open a
public issue**.

**How to report:** use GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a vulnerability** (direct link: <https://github.com/helenkwok/rvt-open/security/advisories/new>). The report stays private between you and the maintainer until an advisory is published, for coordinated disclosure.

Include, if possible:

1. A minimal reproducer (the smallest input that triggers the issue).
2. The exact `rvt-open` version (`cargo pkgid rvt`).
3. Your platform and Rust toolchain version.
4. A description of the impact (denial of service, memory safety,
   silent data corruption, information disclosure, etc.).
5. For parser crashes, rerun the failing command or API call with
   `RUST_BACKTRACE=1` and include the backtrace, the command line, and
   whether the input was passed through the CLI, Rust API, Python API,
   Wasm API, or viewer.

Reports are handled on a best-effort basis: there is no guaranteed
response or fix time. You can expect the report to be read, an
acknowledgement when it has been, a candid assessment of whether it is in
scope, and a fix in the next release for a confirmed issue, with more
severe issues taken first. If a report seems to have gone unnoticed, a
short follow-up on the same advisory is welcome.

## Scope

In scope:

- **Malformed input parsing.** Any input file — valid RVT/RFA or
  not — should not crash the library, trigger `panic!`, or cause
  out-of-bounds reads. If you find one that does, that's a bug.
- **Information disclosure via output.** If rvt-open's default
  output (without `--redact`) leaks more than the input file
  itself contains, that's a bug.
- **Memory safety.** The core library crate has
  `#![forbid(unsafe_code)]` (no `unsafe` in our parser code today;
  any `unsafe` that slips in must have a safety argument). Python
  bindings live in the separate `rvt-py` crate.
- **Denial of service via resource exhaustion** (file-size-
  linear CPU/memory only). A file ten times the size should not
  cause a hundred-times-larger allocation.

Out of scope:

- Bugs in our runtime dependencies (`cfb`, `flate2`, etc.) that
  have been reported to those projects and are under their own
  security policies.
- Issues in the Revit file format itself (Autodesk's problem).
- Redistribution of Autodesk-owned sample files (please don't send
  us any — the test corpus is pulled from phi-ag/rvt at build
  time).

## Disclosure

Once a fix ships, I'll credit the reporter (if they want credit) in
the CHANGELOG and in a security advisory. The disclosure timeline is agreed with
the reporter; by default the advisory is published once the fix is in a
release.

## Verifying a release

Every release on the [Releases page](https://github.com/helenkwok/rvt-open/releases) is built by the
[release workflow](.github/workflows/release.yml) from the tagged commit, on GitHub-hosted runners. Each
archive (the CLI archives and the WebAssembly tarball) comes with:

| Asset | What it lets you check |
|---|---|
| Build provenance attestation | The archive was built by this repository's release workflow from the tagged commit. |
| `SHA256SUMS` | The file you downloaded is the file that was published. |
| `<archive>.cdx.json` | A CycloneDX SBOM: every Rust crate compiled into that archive, with its version. |
| `<archive>.trivy.txt` | A Trivy vulnerability report on that SBOM, made when the release was built. |
| `scans.md` | Malware scan results for every archive, in one file: ClamAV, and the VirusTotal and MetaDefender Cloud report links. |
| `clamav.log` | ClamAV's full output, including the engine and signature database versions. |

Check the provenance with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify rvt-open-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz -R helenkwok/rvt-open
```

Check the checksum, in the folder holding the download and `SHA256SUMS`:

```sh
sha256sum --check --ignore-missing SHA256SUMS      # Linux
shasum -a 256 --check --ignore-missing SHA256SUMS  # macOS
```

On Windows, compare `Get-FileHash <file> -Algorithm SHA256` with the line in `SHA256SUMS`.

To check the dependencies yourself, scan the SBOM with any tool that reads CycloneDX, for example
`trivy sbom <archive>.cdx.json` or `osv-scanner --sbom <archive>.cdx.json`. A VirusTotal link opens the
report for that exact file (it is keyed by the SHA-256).

`scans.md` has one section per scanner:

- **ClamAV** scans every archive and every file unpacked from it on the release runner, from the official
  `clamav/clamav` image pinned by digest. The image carries its own signature database, and freshclam
  fetches newer signatures first when it can; the section records the engine and database version used.
  It always runs, and the release is not published if it flags anything. ClamAV is used as a CI tool
  only and is not part of rvt-open.
- **VirusTotal** and **MetaDefender Cloud** (OPSWAT, multi-engine) each get every archive uploaded, and
  their verdicts and report links are listed.
- **Hybrid Analysis** (CrowdStrike Falcon Sandbox) gives each archive a quick scan, and the Windows, Linux
  x86_64 and macOS arm64 archives a sandbox run. Its results and report links are printed in the release
  workflow's log (the Actions run for the tag), not in `scans.md`, which only notes that it ran.
- These three run only when the service's key is configured, and a service being unavailable does not
  stop a release, so a section can say "not run" or "upload failed". The uploads are public on each
  service.

A clean result from any scanner is evidence, not proof: compare it with the provenance check above.

The repository itself is scanned on every change by CodeQL, cargo-deny (RustSec advisories, licences and
crate sources), Trivy and the OpenSSF Scorecard; the results are in the Security tab.
