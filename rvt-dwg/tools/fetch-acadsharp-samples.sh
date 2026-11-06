#!/usr/bin/env bash
# Fetch the ACadSharp sample drawings the rvt-dwg tests read, pinned by commit and SHA-256.
#
#   rvt-dwg/tools/fetch-acadsharp-samples.sh <dir>     then: RVT_DWG_SAMPLES=<dir> cargo test -p rvt-dwg
#
# ACadSharp (https://github.com/DomCR/ACadSharp) is MIT licensed, Copyright (c) 2021 Albert Domenech;
# its LICENSE covers samples/. The files are downloaded at test time and not committed.
set -euo pipefail
dir=${1:?usage: fetch-acadsharp-samples.sh <dir>}
commit=d9f3de8c9910b602d392aa1acc72b62c095ec40e
base=https://raw.githubusercontent.com/DomCR/ACadSharp/$commit/samples
mkdir -p "$dir"
while read -r sum name; do
  f="$dir/$name"
  if [[ -f $f ]] && echo "$sum  $f" | sha256sum -c --status 2>/dev/null; then continue; fi
  curl -sfL --retry 3 -o "$f.part" "$base/$name"
  echo "$sum  $f.part" | sha256sum -c --quiet || { echo "FAIL: $name does not match its pinned SHA-256" >&2; rm -f "$f.part"; exit 1; }
  mv "$f.part" "$f"
done <<'SUMS'
d1ec4519be14ac49ecab5ec024ad951271e00261a1851f623a784a97ba6ede73 sample_AC1014.dwg
8ffbdc713f5838cbbb1596a8422e2b607e5c9e79921e852cbb5abee4af76ea4e sample_AC1015.dwg
7d26f908516ec5fb89cc8054af33aff233b68a7b63208bbc5b7f9f4d9bff9866 sample_AC1018.dwg
70b5efec9a3c230a9892e810154a760fe72311b1e730fb582f22950135da770a sample_AC1021.dwg
53ecc28e54ff0bbfb022989c47fd1cb16eb07a98825309b66cdf316d900fe4ce sample_AC1024.dwg
4c63508dd794d5cdfb56c60aded47928999b02619ea8c2ef435a4b74e60d24d6 sample_AC1027.dwg
0e8faaca949c9429c92240082d9ea4d3524aca5baf7c7815ec439245c321b528 sample_AC1032.dwg
d0039feca2eda1ead9a7135854c62e0818d0ae0af842a44e1645cf8c8523fd75 sample_AC1009_ascii.dxf
dc8612a2670d8bbb0697d6a4e51b197b6bf2055c08c95b8c646515bb63ca947f sample_AC1009_binary.dxf
4f7d7f0d3957d4d32c6c43e4c600eea8e90fc0fcb1b36b11256095a9789a923f sample_AC1015_ascii.dxf
a3b3d56311371355b0fc01c50c2fa8ac741ed87429114fe9fcdfb3d2ad15d535 sample_AC1015_binary.dxf
71117bafa8f0d9816607768c8fb21676c83f99892c537711a68c24aa7e06e798 sample_AC1018_ascii.dxf
d659b8e7964eb30700660eea34454696bbbfc927726c3417d93df8776e88e225 sample_AC1018_binary.dxf
39ebdd9a48fef7a6240a04e3b5ca6d45ba5bb30b560fb9dadf26774e1af83731 sample_AC1021_ascii.dxf
9650c4f2b44f064e09c8ba10ccf558f9438bf7779bed18e6144887930ddb7cb1 sample_AC1021_binary.dxf
c97e857047ad4cecd84754ea1a8638e47508e339fd489f1e895ee7332127b372 sample_AC1024_ascii.dxf
6a99aed5d669235a12f592d5cce426c6fba29b5462bae8d76f98187a2fdda22f sample_AC1024_binary.dxf
4f26752008643642c37f7bab410b23cfd8b6dd9ee3aaa19746b612ea956b61e6 sample_AC1027_ascii.dxf
9a8c4dc15828f62b587273f41cfac2a63ce7614841844a15a15248b49cab7e14 sample_AC1027_binary.dxf
c2934f616ecd7097e773b85e5d6e0548d21ccc635abcc8e7276f7d4a585a7ae2 sample_AC1032_ascii.dxf
e2c59abecc7bc0fd0ace7b9b6f083d2a16d36077b7a7d7ceccd2aa7edfb348e1 sample_AC1032_binary.dxf
SUMS
echo "ok: ACadSharp samples in $dir"
