# recall.py <docx> <pdf>: fraction of the source's words that appear in the rendered PDF.
# Ligature-proof: pdftotext drops ligature glyphs (fi, fl, ff, ti, ...), so both sides have
# those letter pairs removed and whitespace squashed before matching.
import sys,re,zipfile,subprocess,html
docx,pdf=sys.argv[1],sys.argv[2]
z=zipfile.ZipFile(docx)
LIG=['ffi','ffl','ff','fi','fl','ft','ti','tt','fj','st','ct']
def norm(s):
    s=re.sub(r'[^a-z0-9]','',s.lower())
    for l in LIG: s=s.replace(l,'')
    return s
def words(xml):
    out=[]
    for p in re.findall(r'<w:p[ >].*?</w:p>',xml,re.S):
        t=html.unescape(''.join(re.findall(r'<w:t(?: [^>]*)?>([^<]*)</w:t>',p)))
        out+= [w for w in re.findall(r'[A-Za-z0-9]{4,}',t)]
    return out
pdftext=subprocess.run(['pdftotext',pdf,'-'],capture_output=True,text=True).stdout
hay=norm(re.sub(r'-\n','',pdftext))
def score(ws):
    ws=[w for w in ws if len(norm(w))>=3]
    if not ws: return None,[]
    miss=[w for w in ws if norm(w) not in hay]
    return 1-len(miss)/len(ws),miss
b,bm=score(words(z.read('word/document.xml').decode('utf8','ignore')))
hf=[]
for n in z.namelist():
    if re.match(r'word/(header|footer)\d*\.xml',n): hf+=words(z.read(n).decode('utf8','ignore'))
h,hm=score(hf)
print(f"body={b:.3f}" if b is not None else "body=n/a", f"hdrftr={h:.3f}" if h is not None else "", "missing:", ' '.join(sorted(set(bm))[:20]))
