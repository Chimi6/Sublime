import sys,re,json
lines=open(sys.argv[1]).read().split('\n')
i=0
while i<len(lines):
    if lines[i].strip()=='message 0: TSWP.StorageArchive (2001)':
        j=i+1; blk=[]
        while j<len(lines) and not lines[j].startswith('#') and not lines[j].startswith('=='): blk.append(lines[j]); j+=1
        text=None; tables={}; cur=None
        for b in blk:
            m=re.match(r'\s+text: (".*")$',b)
            if m: text=json.loads(re.sub(r"\\u\{([0-9a-fA-F]+)\}", lambda k: "\\u%04x" % int(k.group(1),16) if int(k.group(1),16) < 0x10000 else chr(int(k.group(1),16)), m.group(1)))
            m=re.match(r'    (table_\w+):',b)
            if m: cur=m.group(1); tables[cur]=[]
            m=re.match(r'\s+character_index: (\d+)',b)
            if m and cur: tables[cur].append(int(m.group(1)))
        if text is not None:
            n=len(text.encode('utf-16-le'))//2
            pstarts=[0]+[k+1 for k,c in enumerate(text) if c in '\n\x05' and k+1<n]
            probs=[]
            for t,ix in tables.items():
                if ix!=sorted(set(ix)): probs.append(f'{t} unsorted/dup {ix}')
                if any(x>=n and n>0 for x in ix): probs.append(f'{t} past end {ix} n={n}')
            if 'table_para_style' in tables and tables['table_para_style']!=pstarts: probs.append(f"para {tables['table_para_style']} vs {pstarts}")
            if probs or '-v' in sys.argv: print(repr(text[:70]), probs, tables.get('table_char_style'))
        i=j
    else: i+=1
