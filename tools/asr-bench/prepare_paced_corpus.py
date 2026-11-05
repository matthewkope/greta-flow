"""Construct complete-utterance 2/5/15/30/60s tests with labelled synthetic pauses."""
import hashlib
import json
import sys
from pathlib import Path
import numpy as np
root=Path(sys.argv[1]);smoke=Path(sys.argv[2])
base=[r for r in json.loads((root/'manifest.json').read_text()) if r['condition']=='clean']
rows=[]
for duration in [2,5,15,30,60]:
    audio=[];texts=[];sources=[];remaining=duration-1.1
    if duration==2:
        row=min(json.loads(smoke.read_text()),key=lambda r:r['seconds'])
        assert row['seconds']<=2
        audio.extend(np.fromfile(row['path'],dtype='<f4'));texts.append(row['text']);sources.append(row['id'])
    else:
        for row in sorted(base,key=lambda r:r['seconds'],reverse=True):
            if row['seconds']>remaining:continue
            x=np.fromfile(row['path'],dtype='<f4');audio.extend(x);texts.append(row['text']);sources.append(row['id']);remaining-=len(x)/16000
            if remaining>1.2:audio.extend([0.]*19200);remaining-=1.2
    audio.extend([0.]*(duration*16000-len(audio)))
    path=root/f'paced-{duration}.f32';np.array(audio,dtype='<f4').tofile(path)
    rows.append(dict(id=f'paced-{duration}',path=str(path),text=' '.join(texts),seconds=duration,condition='constructed_pauses' if duration>2 else 'short_smoke',sources=sources,sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
(root/'paced.json').write_text(json.dumps(rows,indent=2)+'\n')
