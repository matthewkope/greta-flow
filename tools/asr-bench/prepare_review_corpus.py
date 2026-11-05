"""Pinned, multi-speaker real speech plus explicitly labelled synthetic stress variants."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import numpy as np
import pyarrow.parquet as pq
import soundfile as sf
from huggingface_hub import hf_hub_download

p=argparse.ArgumentParser()
p.add_argument('directory',type=Path)
a=p.parse_args(); a.directory.mkdir(parents=True,exist_ok=True)
revision='71cacbfb7e2354c4226d01e70d77d5fca3d04ba1'
source=hf_hub_download('openslr/librispeech_asr','clean/test/0000.parquet',repo_type='dataset',revision=revision)
expected='7113aa4c3cf963fb54697145719a7725f984c8836d1c494a554cbb9f1a017df0'
assert hashlib.sha256(Path(source).read_bytes()).hexdigest()==expected
rows=pq.read_table(source).to_pylist()
clips=[]
rng=np.random.default_rng(33)
for idx in np.linspace(0,len(rows)-1,120,dtype=int):
    row=rows[idx]; audio,rate=sf.read(io.BytesIO(row['audio']['bytes']),dtype='float32'); assert rate==16000
    for kind,samples in [('clean',audio),('quiet',audio*0.02),('noise_20db',audio+rng.normal(0,float(np.sqrt(np.mean(audio**2)))*0.1,len(audio)).astype('float32'))]:
        path=(a.directory/f"{row['id']}-{kind}.f32").resolve(); samples.astype('<f4').tofile(path)
        clips.append(dict(id=f"{row['id']}-{kind}",path=str(path),text=row['text'],seconds=len(samples)/rate,speaker_id=row['speaker_id'],condition=kind,sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
(a.directory/'manifest.json').write_text(json.dumps(clips,indent=2)+'\n')
(a.directory/'source.json').write_text(json.dumps(dict(dataset='openslr/librispeech_asr',revision=revision,file='clean/test/0000.parquet',sha256=expected,license='CC-BY-4.0',selection='120 evenly spaced rows; seed 33; quiet=0.02 gain; noise=20dB white noise',speakers=len({c['speaker_id'] for c in clips})),indent=2)+'\n')
print(f"Exported {len(clips)} files from 120 distinct clips, {len({c['speaker_id'] for c in clips})} speakers")
