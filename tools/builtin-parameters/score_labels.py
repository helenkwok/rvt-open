#!/usr/bin/env python3
"""Score the help terms of help-terms.tsv against Revit's own names in DEU, JPN and CHS, and drop the wrong ones.

usage: score_labels.py KEY_DIR [--terms PATH] [--write]   (without --write it exits 1 while a term the key shows wrong remains)

KEY_DIR holds `<model>__<LANG>.params.ndjson.gz` (ENU, DEU, JPN, CHS): the Revit API dump of public
samples run with each UI language, one JSON line per element with its parameters (`builtin`, `id`,
`name`). It is the answer key, never a source: the name Revit shows for a built-in parameter id in a
language is `name` of any parameter with that `id` (the dump shows one name per id and language).
For each of DEU, JPN and CHS it counts, over the ids the key has, the help terms that equal the
key's name (match) and those that do not (differ). With --write it removes each (id, language) row
the key shows wrong from the terms file, so the table holds a term only where nothing shows it wrong;
the other nine languages of an id stay (the languages go wrong independently: of 679 ids, 278 are
wrong in one of the three, 6 in all three). Rows of ids the key has not seen are not checked."""
import argparse, collections, glob, gzip, hashlib, json, os, sys

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('key_dir')
ap.add_argument('--terms', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'inputs', 'help-terms.tsv'))
ap.add_argument('--write', action='store_true')
args = ap.parse_args()
LANGS = ('DEU', 'JPN', 'CHS')
files = sorted(glob.glob(os.path.join(args.key_dir, '*__*.params.ndjson.gz')))
if not files:
    sys.exit(f'no *__<LANG>.params.ndjson.gz in {args.key_dir}')
key = collections.defaultdict(dict)  # id -> lang -> {names}
for fn in files:
    lang = os.path.basename(fn).rsplit('__', 1)[1].split('.')[0]
    if lang not in LANGS:
        continue
    with gzip.open(fn, 'rt', encoding='utf-8') as f:
        for line in f:
            for p in json.loads(line)['params']:
                if p.get('builtin') and isinstance(p.get('id'), int) and p['id'] < 0 and p.get('name'):
                    key[p['id']].setdefault(lang, set()).add(p['name'])
header, rows = [], []
for l in open(args.terms, encoding='utf-8'):
    (header if l.startswith('#') else rows).append(l.rstrip('\n'))
stat = {l: collections.Counter() for l in LANGS}
kept, dropped = [], []
for r in rows:
    i, lang, term = r.split('\t')[:3]
    names = key.get(int(i), {}).get(lang)
    if lang in LANGS and names:
        ok = names == {term}
        stat[lang]['match' if ok else 'differ'] += 1
        if not ok:
            dropped.append(r)
            continue
    kept.append(r)
print('language\tkey_ids\tmatch\tdiffer\tprecision')
for l in LANGS:
    n = stat[l]['match'] + stat[l]['differ']
    print(f"{l}\t{n}\t{stat[l]['match']}\t{stat[l]['differ']}\t{stat[l]['match'] / n:.4f}")
print(f'{len(rows)} rows, {len(dropped)} shown wrong by the key')
if args.write:
    sha = hashlib.sha256(b''.join(open(f, 'rb').read() for f in files)).hexdigest()[:16]
    header = [h for h in header if not h.startswith('# score_labels')]
    header.append(f'# score_labels.py: {len(dropped)} (id, language) rows shown wrong by the 2027 DEU/JPN/CHS dump removed (key files sha {sha})')
    with open(args.terms + '.tmp', 'w', encoding='utf-8') as f:
        f.write('\n'.join(header + kept) + '\n')
    os.replace(args.terms + '.tmp', args.terms)
else:
    sys.exit(1 if dropped else 0)
