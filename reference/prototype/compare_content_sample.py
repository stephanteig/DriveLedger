import os, sys, hashlib, unicodedata as u
A, B = sys.argv[1], sys.argv[2]
skip = {'.DocumentRevisions-V100','.Spotlight-V100','.TemporaryItems','.Trashes','.fseventsd'}
def junk(f): return f=='.DS_Store' or f.startswith('._')
def sample(p, sz):
    h=hashlib.sha1()
    with open(p,'rb') as fh:
        for off in (0, max(0,sz//2-524288), max(0,sz-1048576)):
            fh.seek(off); h.update(fh.read(1048576))
    return h.hexdigest()
bysize={}
for root,dirs,files in os.walk(B):
    dirs[:]=[d for d in dirs if d not in skip]
    for f in files:
        if junk(f): continue
        p=os.path.join(root,f)
        try: bysize.setdefault(os.path.getsize(p),[]).append(p)
        except OSError: pass
found, missing = [], []
for root,dirs,files in os.walk(A):
    for f in files:
        if junk(f): continue
        p=os.path.join(root,f); sz=os.path.getsize(p); rel=os.path.relpath(p,A)
        cands=bysize.get(sz,[])
        hit=None
        if cands and sz>0:
            hs=sample(p,sz)
            for c in cands:
                if sample(c,sz)==hs: hit=c; break
        elif sz==0 and cands:
            hit=next((c for c in cands if u.normalize('NFC',os.path.basename(c))==u.normalize('NFC',f)),None)
        (found.append((rel,sz,hit)) if hit else missing.append((rel,sz)))
renamed=[(r,s,h) for r,s,h in found if u.normalize('NFC',os.path.basename(h))!=u.normalize('NFC',os.path.basename(r))]
print(f"FOUND (content match anywhere on ByHola_1): {len(found)}")
print(f"  of which under a DIFFERENT NAME: {len(renamed)}")
for r,s,h in renamed: print(f"    {r}\n      -> {os.path.relpath(h,B)}")
print(f"\nMISSING: {len(missing)}  total {sum(s for _,s in missing)/1e9:.2f} GB")
for r,s in sorted(missing): print(f"  {r}\t{s/1e6:.1f} MB")
