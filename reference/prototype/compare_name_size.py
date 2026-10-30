import os, sys, unicodedata as u
A, B = sys.argv[1], sys.argv[2]
skip = {'.DocumentRevisions-V100','.Spotlight-V100','.TemporaryItems','.Trashes','.fseventsd'}
n = lambda s: u.normalize('NFC', s)
idx = {}
for root, dirs, files in os.walk(B):
    dirs[:] = [d for d in dirs if d not in skip]
    for f in files:
        if f == '.DS_Store' or f.startswith('._'): continue
        p = os.path.join(root, f)
        try: sz = os.path.getsize(p)
        except OSError: continue
        idx.setdefault(n(f), []).append((sz, p))
exact, size_diff, missing = [], [], []
for root, dirs, files in os.walk(A):
    for f in files:
        if f == '.DS_Store' or f.startswith('._'): continue
        p = os.path.join(root, f); sz = os.path.getsize(p)
        hits = idx.get(n(f), [])
        same = [h for h in hits if h[0] == sz]
        rel = os.path.relpath(p, A)
        if same: exact.append((rel, same[0][1]))
        elif hits: size_diff.append((rel, sz, hits))
        else: missing.append((rel, sz))
print(f"MATCH (same name+size): {len(exact)}")
for r, h in exact: print(f"  {r}\n    -> {os.path.relpath(h, B)}")
print(f"\nSAME NAME, DIFFERENT SIZE: {len(size_diff)}")
for r, sz, hits in size_diff:
    print(f"  {r} ({sz})"); [print(f"    -> {os.path.relpath(p,B)} ({s})") for s,p in hits]
print(f"\nMISSING: {len(missing)}  total {sum(s for _,s in missing)/1e9:.2f} GB")
for r, sz in missing: print(f"  {r} ({sz/1e9:.2f} GB)")
