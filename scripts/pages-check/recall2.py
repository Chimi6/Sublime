# recall2.py <truth.pdf> <test.pdf>: fraction of the truth's words (4+ letters) found in the test PDF.
import sys,re,subprocess
LIG=['ffi','ffl','ff','fi','fl','ft','ti','tt','fj','st','ct']
def norm(s):
    s=re.sub(r'[^a-z0-9]','',s.lower())
    for l in LIG: s=s.replace(l,'')
    return s
def text(p): return subprocess.run(['pdftotext',p,'-'],capture_output=True,text=True).stdout
truth=re.findall(r'[A-Za-z0-9]{4,}',re.sub(r'-\n','',text(sys.argv[1])))
hay=norm(re.sub(r'-\n','',text(sys.argv[2])))
ws=[w for w in truth if len(norm(w))>=3]
if not ws: print("recall=n/a"); sys.exit()
miss=[w for w in ws if norm(w) not in hay]
print(f"recall={1-len(miss)/len(ws):.3f}", "missing:", ' '.join(sorted(set(miss))[:12]))
