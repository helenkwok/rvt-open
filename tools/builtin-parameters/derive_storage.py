#!/usr/bin/env python3
"""The storage type of each built-in parameter id, from the values public files store.

usage: derive_storage.py CENSUS_DIR OUT.tsv

CENSUS_DIR holds one `<model>.tsv.gz` per file from `cargo run --release --example param_census FILE`
(columns: element id, parameter id, origin, type, value, class). Origin S is the element's own value
sets, N its other parameter sets, F what `field_parameters` derives from object fields. Only S and N
are read: a type is a file's own, not an interpretation. An id seen with two types takes the more common
one and the other counts are kept.
OUT.tsv: id, type, models, counts (type:n;...), and a second file OUT.f-only.tsv lists the ids only F gives."""
import collections, gzip, os, sys
d, out = sys.argv[1:3]
sn = collections.defaultdict(collections.Counter)
models = collections.defaultdict(set)
fonly = collections.defaultdict(collections.Counter)
for fn in sorted(os.listdir(d)):
    if not fn.endswith('.tsv.gz'):
        continue
    with gzip.open(os.path.join(d, fn), 'rt', encoding='utf-8', errors='replace') as g:
        for line in g:
            p = line.rstrip('\n').split('\t')
            if len(p) < 4:
                continue
            try:
                pid = int(p[1])
            except ValueError:
                continue
            if pid >= 0:
                continue
            if p[2] in 'SN':
                sn[pid][p[3]] += 1
                models[pid].add(fn)
            else:
                fonly[pid][p[3]] += 1
with open(out, 'w') as f:
    f.write('# id\ttype\tmodels\tcounts\n')
    for i in sorted(sn):
        c = sn[i]
        f.write(f"{i}\t{c.most_common(1)[0][0]}\t{len(models[i])}\t{';'.join(f'{t}:{n}' for t, n in c.most_common())}\n")
with open(out.replace('.tsv', '.f-only.tsv'), 'w') as f:
    f.write('# id\tcounts (origin F only)\n')
    for i in sorted(fonly):
        if i not in sn:
            f.write(f"{i}\t{';'.join(f'{t}:{n}' for t, n in fonly[i].most_common())}\n")
