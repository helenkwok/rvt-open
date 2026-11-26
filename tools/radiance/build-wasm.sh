#!/usr/bin/env bash
# Build Radiance's gensky, gendaylit, oconv and rtrace as WebAssembly for the viewer's validated daylight tier.
#
#   tools/radiance/build-wasm.sh OUT_DIR
#
# Radiance is the commit the 6.0.3 release was built from (84f888ec), fetched from GitHub and checked against its
# sha256, then patched with tools/radiance/o_default.patch (a function signature WebAssembly's indirect calls
# check). It is compiled with Emscripten 7.0.1 in its official image, pinned by digest. Two build settings differ
# from a native build: Emscripten's libm is part of its libc (an empty libm.a satisfies the LIB_M check), and the
# x86-only -mavx2 -mfma flags are left out. Memory grows to 4 GiB (WebAssembly's 32-bit limit; Emscripten's default
# stops at 2 GiB), which oconv needs for large scenes. Each program is an ES module (MODULARIZE, EXPORT_ES6) with its own
# in-memory file system, run with callMain.
#
# OUT_DIR receives <program>.js and <program>.wasm for each program, the files those programs read at run time
# (rayinit.cal, skybright.cal, perezlum.cal), Radiance's License.txt and SOURCE.md. Needs docker and curl.
# RADIANCE_DOCKER_ARGS adds options to the docker run (limits, a name). Runs on Linux (GNU sha256sum and sed), as CI does.
set -euo pipefail
out=${1:?usage: build-wasm.sh OUT_DIR}
root=$(cd "$(dirname "$0")/../.." && pwd)
COMMIT=84f888ec659b8a28f7f8841e05e9ced34899b653
SHA=6d7b440ddbcb15f095b54d701ed991d358abb19e1c590ce0a2c2b40a37058c30
IMAGE=emscripten/emsdk:7.0.1-x64@sha256:a6e0e2f5f18d89c9d94e67606e5e973fddcc26487ccb4dc82eb7dc3d8994873c
PROGRAMS=(gensky gendaylit oconv rtrace)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 --retry-all-errors --retry-delay 5 -o "$work/radiance.tar.gz" "https://codeload.github.com/LBNL-ETA/Radiance/tar.gz/$COMMIT"
echo "$SHA  $work/radiance.tar.gz" | sha256sum -c --quiet -
mkdir "$work/src"
tar -xzf "$work/radiance.tar.gz" -C "$work/src" --strip-components=1
(cd "$work/src" && patch -p1 < "$root/tools/radiance/o_default.patch")
sed -i 's/^\(\s*\)add_compile_options(-mavx2 -mfma)/\1# WebAssembly: no AVX2 or FMA/' "$work/src/CMakeLists.txt"
grep -q 'WebAssembly: no AVX2 or FMA' "$work/src/CMakeLists.txt" || { echo "CMakeLists.txt: the -mavx2 -mfma line was not found" >&2; exit 1; }
cat > "$work/build.sh" <<'SH'
set -euo pipefail
cd /w
echo "int radiance_wasm_libm_stub;" > stub.c && emcc -c stub.c -o stub.o && emar rcs libm.a stub.o
mkdir build && cd build
LF="-sALLOW_MEMORY_GROWTH=1 -sMAXIMUM_MEMORY=4GB -sSTACK_SIZE=4MB -sEXIT_RUNTIME=1 -sMODULARIZE=1 -sEXPORT_ES6=1 -sENVIRONMENT=web,worker,node -sFORCE_FILESYSTEM=1 -sINVOKE_RUN=0 -sEXPORTED_RUNTIME_METHODS=callMain,FS"
emcmake cmake ../src -DBUILD_HEADLESS=ON -DLIB_M=/w/libm.a -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_FLAGS="-O3 -Wno-everything" -DCMAKE_CXX_FLAGS="-O3 -Wno-everything" -DCMAKE_EXE_LINKER_FLAGS="$LF" > ../cmake.log 2>&1 || { tail -40 ../cmake.log; exit 1; }
emmake make -j"$(nproc)" "$@" > ../make.log 2>&1 || { grep -n -i -B2 -A6 error ../make.log | head -80; exit 1; }
emcc --version | head -1 > ../emcc-version.txt
SH
docker run --rm ${RADIANCE_DOCKER_ARGS:-} -v "$work:/w" -u "$(id -u):$(id -g)" -e HOME=/tmp "$IMAGE" bash /w/build.sh "${PROGRAMS[@]}"
mkdir -p "$out"
for p in "${PROGRAMS[@]}"; do
  cp "$work/build/bin/$p.js" "$work/build/bin/$p.wasm" "$out/"
done
cp "$work/src/src/rt/rayinit.cal" "$work/src/src/gen/skybright.cal" "$work/src/src/gen/perezlum.cal" "$out/"
cp "$work/src/License.txt" "$out/License.txt"
cat > "$out/SOURCE.md" <<MD
# Radiance, compiled to WebAssembly

gensky, gendaylit, oconv and rtrace from Radiance (https://github.com/LBNL-ETA/Radiance), commit $COMMIT
(the sources of the 6.0.3 release), compiled with $(cat "$work/emcc-version.txt") (image $IMAGE).

Radiance is Copyright (c) 1990 to 2025, The Regents of the University of California, through Lawrence Berkeley
National Laboratory, under the Radiance Software License, Version 2.0 (License.txt beside this file).

One change to the sources, in rvt-open's tools/radiance/o_default.patch: o_default() in src/ot/initotypes.c and
src/ot/init2otypes.c declares the two parameters oconv calls it with, because WebAssembly checks the signature of
an indirect call. Build settings: an empty libm.a for the LIB_M check (Emscripten's libm is in its libc), no
-mavx2 -mfma, and memory that grows to 4 GiB.
MD
(cd "$out" && sha256sum ./*.js ./*.wasm ./*.cal > SHA256SUMS)
ls -l "$out"
