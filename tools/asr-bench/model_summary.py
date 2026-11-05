"""Score paired model runs without counting repetitions as new accuracy samples."""
import argparse
import json
import math
import re
from collections import Counter
from pathlib import Path

import numpy as np
from summarize import distance, normalize_english
from punctuation import score as punctuation_score


def protected_count(text, value, numeric=False):
    # Exact spelling/case and digit formatting matter here; WER normalizes them away.
    prefix = r"(?<!\w)(?<!\d[.,])" if numeric else r"(?<!\w)"
    suffix = r"(?!\w|[.,]\d)" if numeric else r"(?!\w)"
    return len(re.findall(prefix + re.escape(value) + suffix, text))


def summarize(manifest, rows, repeats=2):
    if repeats < 1:
        raise ValueError('At least one warm pass is required')
    refs = {c['id']: c for c in manifest}
    if not refs or len(refs) != len(manifest):
        raise ValueError('Empty manifest or duplicate clip IDs')
    calls = [r for r in rows if r.get('event') == 'transcribe']
    expected = {(i, p) for i in refs for p in range(1, repeats + 1)}
    warm = [r for r in calls if r.get('pass', 0) > 0]
    if Counter((r['id'], r['pass']) for r in warm) != Counter(expected):
        raise ValueError('Incomplete, duplicate, or unexpected warm pass')
    first = [r for r in calls if r.get('pass') == 0]
    loads = [r for r in rows if r.get('event') == 'load']
    if len(first) != 1 or first[0]['id'] not in refs or len(loads) != 1:
        raise ValueError('Expected exactly one load and one first call')
    for row in calls + loads:
        if not math.isfinite(row['seconds']) or row['seconds'] < 0:
            raise ValueError('Invalid latency')
    result = {'load_seconds': loads[0]['seconds'], 'first_call_ms': first[0]['seconds'] * 1000,
              'conditions': {}}
    for condition in sorted({c.get('condition', 'clean') for c in manifest}):
        timed = [r for r in warm if refs[r['id']].get('condition', 'clean') == condition]
        scored = [r for r in timed if r['pass'] == 1]
        word_errors = word_count = 0
        metrics = {k: {'expected': 0, 'missing': 0, 'extra': 0} for k in ['names', 'numbers']}
        punct_errors = punct_count = punct_clips = 0
        sentence_errors = sentence_marks = sentence_exact = 0
        for row in scored:
            ref = refs[row['id']]
            a, b = normalize_english(ref['text']).split(), normalize_english(row['text']).split()
            word_errors += distance(a, b)
            word_count += len(a)
            for kind in metrics:
                for value in set(ref.get(kind, [])):
                    n, m = protected_count(ref['text'], value, kind == 'numbers'), protected_count(row['text'], value, kind == 'numbers')
                    if n == 0:
                        raise ValueError(f'{kind} annotation absent from reference: {value}')
                    metrics[kind]['expected'] += n
                    metrics[kind]['missing'] += max(n - m, 0)
                    metrics[kind]['extra'] += max(m - n, 0)
            if ref.get('score_punctuation', False):
                # Sequence edit rate, not a claim that punctuation occurs at the correct word boundary.
                a, b = re.findall(r'[^\w\s]', ref['text']), re.findall(r'[^\w\s]', row['text'])
                punct_errors += distance(a, b)
                punct_count += len(a)
                punct_clips += 1
                sentence = punctuation_score(ref['text'], row['text'])
                sentence_errors += sentence['errors']
                sentence_marks += sentence['reference_marks']
                sentence_exact += sentence['errors'] == 0
        result['conditions'][condition] = {
            'clips': len(scored), 'warm_calls': len(timed), 'word_errors': word_errors,
            'reference_words': word_count, 'wer_percent': 100 * word_errors / word_count if word_count else None,
            'p50_ms': float(np.percentile([r['seconds'] for r in timed], 50) * 1000),
            'p95_ms': float(np.percentile([r['seconds'] for r in timed], 95) * 1000),
            'names': metrics['names'] if metrics['names']['expected'] else None,
            'numbers': metrics['numbers'] if metrics['numbers']['expected'] else None,
            'punctuation': {'clips': punct_clips, 'errors': punct_errors, 'reference_marks': punct_count,
                            'sequence_error_percent': 100 * punct_errors / punct_count if punct_count else None,
                            'sentence_boundary_errors': sentence_errors, 'sentence_reference_marks': sentence_marks,
                            'sentence_exact_clips': sentence_exact,
                            'sentence_error_percent': 100 * sentence_errors / sentence_marks if sentence_marks else None}
                           if punct_clips else None,
            'transcript_variants': sum(len({r['text'] for r in timed if r['id'] == s['id']}) > 1 for s in scored),
        }
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path)
    parser.add_argument('results', nargs='+', type=Path)
    parser.add_argument('--repeats', type=int, default=2)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text())
    output = {str(p): summarize(manifest, [json.loads(s) for s in p.read_text().splitlines()
                                          if s.startswith('{')], args.repeats) for p in args.results}
    print(json.dumps(output, indent=2))
