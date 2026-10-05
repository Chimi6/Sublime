# btdecode.py <log> <assertion-number> [frames]: decodes a Pages "#Assert ... Assertion backtrace: >>...<<"
# line (base64 -> xz -> image UUIDs and frame addresses) and symbolicates the frames against the
# frameworks in /Applications/Pages.app with nm, to find the code path that logged it.
import re,sys,base64,lzma,struct,subprocess,glob,bisect,os
log=open(sys.argv[1]).read(); want=sys.argv[2]
bt=dict(re.findall(r'Assertion failure #(\d+): Assertion backtrace: >>([A-Za-z0-9+/=]+)<<',log))
raw=base64.b64decode(bt[want]); d=lzma.decompress(raw[raw.find(b'\xfd7zXZ'):])
nimg,nfr=struct.unpack_from('<QQ',d,0); off=16; imgs=[]
for i in range(nimg):
    u=d[off:off+16].hex(); base,=struct.unpack_from('<Q',d,off+16); off+=24; imgs.append((u,base))
frames=[struct.unpack_from('<Q',d,off+8*i)[0] for i in range(nfr)]
cands=[c for c in glob.glob('/Applications/Pages.app/Contents/Frameworks/*.framework/Versions/A/*')+['/Applications/Pages.app/Contents/MacOS/Pages'] if os.path.isfile(c)]
uu={}
for c in cands:
    for line in subprocess.run(['dwarfdump','--uuid',c],capture_output=True,text=True).stdout.splitlines():
        if '(arm64)' in line: uu[line.split()[1].replace('-','').lower()]=c
cache={}
def sym(p,o):
    if p not in cache:
        s=[]
        for l in subprocess.run(['nm','-arch','arm64','-n','--defined-only',p],capture_output=True,text=True).stdout.splitlines():
            q=l.split(' ',2)
            if len(q)==3:
                try: s.append((int(q[0],16),q[2]))
                except: pass
        s.sort(); cache[p]=(s,[a for a,_ in s])
    s,a=cache[p]; i=bisect.bisect_right(a,o)-1; return s[i][1] if i>=0 else '?'
for f in frames[:int(sys.argv[3]) if len(sys.argv)>3 else 18]:
    best=max(((u,b) for u,b in imgs if b<=f), key=lambda x:x[1])
    p=uu.get(best[0]); print(f"{os.path.basename(p) if p else best[0][:8]:16} {hex(f-best[1])} {sym(p,f-best[1]) if p else ''}")
