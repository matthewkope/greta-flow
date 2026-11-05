"""Download only the pinned benchmark files and verify every byte against the manifest."""
import argparse
import hashlib
import json
from pathlib import Path
from huggingface_hub import hf_hub_download


def digest(path):
    with path.open('rb') as handle:
        return hashlib.file_digest(handle, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path, help='Local cache outside Git')
    parser.add_argument('--manifest', type=Path, default=Path(__file__).with_name('model-manifest-0.4.json'))
    parser.add_argument('--model', action='append', help='Only these model IDs (repeatable)')
    parser.add_argument('--verify-only', action='store_true')
    args = parser.parse_args()
    models = json.loads(args.manifest.read_text())
    if args.model and set(args.model) - {m['id'] for m in models}:
        parser.error('Unknown model ID')
    for model in models:
        if args.model and model['id'] not in args.model:
            continue
        for file in model['files']:
            path = args.directory / model['id'] / file['path']
            if not args.verify_only:
                path = Path(hf_hub_download(model['repo'], file['path'], revision=model['revision'],
                                           local_dir=args.directory / model['id']))
            if path.stat().st_size != file['bytes'] or digest(path) != file['sha256']:
                raise ValueError(f'Artifact mismatch: {path}')
        print(f"Verified {model['id']} @ {model['revision']}")


if __name__ == '__main__':
    main()
