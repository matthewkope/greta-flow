"""Score review runs; repeats improve timing estimates, not independent accuracy evidence."""
import json
import sys
from pathlib import Path
import numpy as np
from summarize import normalize_english,distance
manifest=json.loads(Path(sys.argv[1]).read_text())
rows=[json.loads(s) for s in Path(sys.argv[2]).read_text().splitlines() if s.startswith('{')]
refs={c['id']:normalize_english(c['text']).split() for c in manifest}
out={}
for event in {r.get('event') for r in rows} & {'batch','segmented_offline','paced'}:
    for pass_number in [1,2]:
        records=[r for r in rows if r.get('event')==event and r.get('pass')==pass_number]
        if len(records)!=len(refs) or {r['id'] for r in records}!=set(refs):
            raise ValueError(f"Incomplete/duplicate {event} pass {pass_number}")
for event in ['batch','segmented_offline','paced']:
    for condition in sorted({c.get('condition','clean') for c in manifest}):
        ids={c['id'] for c in manifest if c.get('condition','clean')==condition}
        warm=[r for r in rows if r.get('event')==event and r.get('pass',0)>0 and r['id'] in ids]
        if not warm:continue
        first=[r for r in warm if r['pass']==1]
        errors=sum(distance(refs[r['id']],normalize_english(r['text']).split()) for r in first)
        words=sum(len(refs[r['id']]) for r in first)
        out[f'{event}:{condition}']=dict(clips=len(first),calls=len(warm),errors=errors,words=words,wer_percent=100*errors/words if words else None,p50_ms=float(np.percentile([r['seconds'] for r in warm],50)*1000),p95_ms=float(np.percentile([r['seconds'] for r in warm],95)*1000),fallbacks=sum(r.get('fallback',False) for r in warm))
print(json.dumps(out,indent=2))
