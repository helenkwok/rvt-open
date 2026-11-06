#!/usr/bin/env python3
"""affected-tests.py <repo> <commit> [base]: which integration-test modules (tests/<module>.rs) a change can affect.

Prints FULL, NONE (no test can be affected: format, lint, build and box check only) or module names, one per line.
Used by tools/ci/fast-check.sh (pre-merge fast check; FULL there means the smoke set) and the scoped cloud gate.

Selection, from the diff merge-base(base, commit)..commit:
- a test module whose own file changed;
- a changed source file's module: its name, its module path (ifc::export) and the items whose definitions changed,
  looked up in the test texts; then the names of the source files that use it (the crate's module graph, two levels),
  kept only when at most three modules name them;
- a changed binary (src/bin/<x>.rs), or a source file that binary uses, selects the modules that run it
  (CARGO_BIN_EXE_<name>); a changed example selects the modules that name it;
- a changed fixture selects the modules that name the fixture or its directory.
A token most modules mention says nothing and is ignored. FULL when build-wide inputs change (manifests, lockfile, cargo
config, toolchain, the shared test harness, the test runner scripts), when a library source change matches no module, or
when more than half the modules are selected. Changes to rvt-py, rvt-wasm, benches and fuzz select no module (they
are compiled by the fast check; no integration test runs them).
"""
import os
import re
import subprocess
import sys

FULL_FILES = ("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain", "rust-toolchain.toml", "clippy.toml",
              "rustfmt.toml", ".rustfmt.toml", "deny.toml")
FULL_PREFIXES = (".cargo/", "src/lib.rs", "tools/ci/verify-real-files.sh", "tools/check-local.sh", "tools/ci/fast-check.sh",
                 "tests/common", "tests/integration/")
NO_TEST_PREFIXES = ("rvt-py/", "rvt-wasm/", "benches/", "fuzz/", "tools/stream_evidence/")
STOP = {"main", "mod", "new", "test", "tests", "lib", "default", "from", "into", "self", "super", "crate", "error"}


def git(repo, *a):
    return subprocess.run(["git", "-C", repo, *a], capture_output=True, text=True, check=True).stdout


def modpath(f):
    """src/ifc/export.rs -> ifc::export; src/ifc/mod.rs -> ifc."""
    p = f[len("src/"):-3].split("/")
    if p[-1] == "mod":
        p = p[:-1]
    return "::".join(p)


def main():
    repo, commit = sys.argv[1], sys.argv[2]
    base = sys.argv[3] if len(sys.argv) > 3 else "main-next"
    try:
        mb = git(repo, "merge-base", base, commit).strip()
        changed = [f for f in git(repo, "diff", "--name-only", mb, commit).splitlines() if f]
    except subprocess.CalledProcessError:
        print("FULL"); return
    why = [f for f in changed if os.path.basename(f) in FULL_FILES or f.startswith(FULL_PREFIXES)]
    if why:
        print("full: " + ", ".join(why[:5]), file=sys.stderr); print("FULL"); return

    tree = git(repo, "ls-tree", "-r", "--name-only", commit).splitlines()
    modules = sorted(f[len("tests/"):-3] for f in tree if f.startswith("tests/") and f.count("/") == 1 and f.endswith(".rs"))
    show = lambda f: subprocess.run(["git", "-C", repo, "show", f"{commit}:{f}"], capture_output=True, text=True).stdout
    texts = {m: show(f"tests/{m}.rs") for m in modules}
    limit = max(3, len(modules) // 4)

    def mentioning(tok, limit=limit):
        ms = {m for m, t in texts.items() if re.search(r"(?<![\w-])" + re.escape(tok) + r"(?![\w-])", t)}
        return ms if len(ms) <= limit else set()

    rust = [f for f in changed if f.endswith(".rs") and not f.startswith(NO_TEST_PREFIXES)]
    fixtures = [f for f in changed if f.startswith("tests/fixtures/")]
    if not rust and not fixtures:
        print("NONE"); return

    # the crate's module graph: which source files name which modules (use crate::a::b, a::b::, mod b;)
    srcs = [f for f in tree if f.startswith("src/") and f.endswith(".rs")]
    stext = {}
    def src_text(f):
        if f not in stext:
            stext[f] = show(f)
        return stext[f]
    def users(f):
        mp = modpath(f); name = mp.split("::")[-1]
        pat = re.compile(r"\b" + re.escape(mp) + r"\b" + "|" + r"\b(?:super|crate)::" + re.escape(name) + r"\b")
        return {g for g in srcs if g != f and pat.search(src_text(g))}

    chosen, tokens, weak, bins = set(), set(), set(), set()
    for f in rust:
        if f.startswith("tests/") and f.count("/") == 1:
            chosen.add(os.path.basename(f)[:-3]); continue
        if f.startswith("examples/"):
            chosen |= mentioning(os.path.basename(f)[:-3]); continue
        if f.startswith("src/bin/"):
            bins.add(os.path.basename(f)[:-3].replace("_", "-")); continue
        if not f.startswith("src/"):
            tokens.add(os.path.basename(f)[:-3]); continue
        mp = modpath(f)
        tokens.update({mp, mp.split("::")[-1]})
        diff = git(repo, "diff", "-U0", mb, commit, "--", f)
        tokens.update(re.findall(r"\b(?:fn|struct|enum|trait|const|static|type)\s+([A-Za-z_][A-Za-z0-9_]{3,})", diff))
        # two levels of users: their module names are weaker hints, kept only when few modules name them
        frontier = {f}
        for _ in range(2):
            frontier = set().union(*(users(g) for g in frontier)) if frontier else set()
            for g in frontier:
                if g.startswith("src/bin/"):
                    bins.add(os.path.basename(g)[:-3].replace("_", "-"))
                else:
                    weak.add(modpath(g).split("::")[-1])
    tokens -= STOP; weak -= STOP | tokens
    for b in bins:
        chosen |= {m for m, t in texts.items() if "CARGO_BIN_EXE_" + b + '"' in t or "CARGO_BIN_EXE_" + b + ")" in t
                   or re.search(r"CARGO_BIN_EXE_" + re.escape(b) + r"(?![\w-])", t)}
    for f in fixtures:
        rel = f[len("tests/fixtures/"):]
        tokens.add(os.path.basename(f))
        if "/" in rel:
            tokens.add("tests/fixtures/" + rel.split("/")[0])
    for tok in tokens:
        chosen |= mentioning(tok)
    for tok in weak:  # a user's name is a weak hint: only when it points at a few modules
        chosen |= mentioning(tok, 3)

    lib_changed = any(f.startswith("src/") and not f.startswith("src/bin/") for f in rust)
    if (lib_changed and not chosen) or len(chosen) > len(modules) / 2:
        print(f"full: {len(chosen)} of {len(modules)} modules selected", file=sys.stderr); print("FULL"); return
    print("\n".join(sorted(chosen)) if chosen else "NONE")


if __name__ == "__main__":
    main()
