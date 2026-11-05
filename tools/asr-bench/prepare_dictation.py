"""Convert explicitly supplied, locally recorded 16 kHz WAVs to benchmark PCM; no upload."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import soundfile as sf


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('annotations', type=Path, help='JSON rows: id, path, text, names, numbers, score_punctuation')
    parser.add_argument('directory', type=Path, help='Private output directory outside Git')
    args = parser.parse_args()
    annotations = json.loads(args.annotations.read_text())
    if not annotations or len({r['id'] for r in annotations}) != len(annotations):
        parser.error('Empty annotations or duplicate IDs')
    args.directory.mkdir(parents=True, exist_ok=True)
    manifest = []
    rng = np.random.default_rng(33)
    for index, row in enumerate(annotations):
        source = (args.annotations.parent / row['path']).resolve()
        audio, rate = sf.read(source, dtype='float32')
        if rate != 16000 or audio.ndim != 1 or not len(audio) or not np.isfinite(audio).all():
            raise ValueError(f'{source}: expected nonempty finite mono 16 kHz audio')
        if not row.get('text') or not isinstance(row.get('names'), list) or not isinstance(row.get('numbers'), list):
            raise ValueError('Each row needs a checked transcript and names/numbers lists (possibly empty)')
        rms = float(np.sqrt(np.mean(audio ** 2)))
        for condition, samples in [('clean', audio), ('quiet', audio * 0.02),
                                   ('noise_20db', audio + rng.normal(0, rms * 0.1, len(audio)).astype('float32'))]:
            # Numeric file names avoid turning an annotation ID into an arbitrary filesystem path.
            path = (args.directory / f'{index:03d}-{condition}.f32').resolve()
            if path.exists():
                raise FileExistsError(f'Refusing to overwrite {path}; use a new output directory')
            samples.astype('<f4').tofile(path)
            manifest.append({**row, 'id': f"{row['id']}-{condition}", 'path': str(path),
                             'condition': condition, 'seconds': len(audio) / rate,
                             'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                             'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
    (args.directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(f'Prepared {len(manifest)} local files. Keep this directory and its results out of Git.')


if __name__ == '__main__':
    main()
