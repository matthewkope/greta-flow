import copy
import unittest
from model_summary import summarize, protected_count


class ModelSummaryTests(unittest.TestCase):
    def setUp(self):
        self.manifest = [dict(id='a', text='Matt uses Core ML 2.5.', names=['Matt', 'Core ML'],
                              numbers=['2.5'], score_punctuation=True)]
        self.rows = [dict(event='load', seconds=1)] + [
            dict(event='transcribe', id='a', **{'pass': p}, seconds=0.1,
                 text='Matt uses Core ML 2.5.') for p in range(3)]

    def test_repeats_are_not_independent_accuracy(self):
        self.rows[-1]['text'] = 'wrong'
        out = summarize(self.manifest, self.rows)['conditions']['clean']
        self.assertEqual(out['wer_percent'], 0)
        self.assertEqual(out['clips'], 1)
        self.assertEqual(out['warm_calls'], 2)
        self.assertEqual(out['transcript_variants'], 1)

    def test_missing_duplicate_and_unexpected_pass(self):
        for rows in [self.rows[:-1], self.rows + [self.rows[-1]], self.rows[:2],
                     self.rows + [dict(self.rows[-1], **{'pass': 3})]]:
            with self.assertRaises(ValueError):
                summarize(self.manifest, rows)

    def test_missing_first_call_and_load(self):
        for rows in [self.rows[1:], [self.rows[0]] + self.rows[2:]]:
            with self.assertRaises(ValueError):
                summarize(self.manifest, rows)

    def test_names_numbers_and_punctuation_are_separate(self):
        self.rows[2]['text'] = 'Matthew uses core ml twenty five'
        out = summarize(self.manifest, self.rows)['conditions']['clean']
        self.assertEqual(out['names']['missing'], 2)
        self.assertEqual(out['numbers']['missing'], 1)
        self.assertEqual(out['punctuation']['errors'], 2)

    def test_number_is_not_part_of_another_version_or_decimal(self):
        self.assertEqual(protected_count('2.5.1 and 12.5 and 2.50', '2.5', True), 0)
        self.assertEqual(protected_count('Version 2.5.', '2.5', True), 1)
        self.assertEqual(protected_count('1,234', '234', True), 0)

    def test_unannotated_is_not_a_pass(self):
        manifest = [dict(id='a', text='Matt uses Core ML 2.5.')]
        out = summarize(manifest, self.rows)['conditions']['clean']
        for field in ['names', 'numbers', 'punctuation']:
            self.assertIsNone(out[field])

    def test_bad_timing_and_annotations(self):
        rows = copy.deepcopy(self.rows)
        rows[2]['seconds'] = float('nan')
        with self.assertRaises(ValueError):
            summarize(self.manifest, rows)
        self.manifest[0]['names'] = ['missing']
        with self.assertRaises(ValueError):
            summarize(self.manifest, self.rows)


if __name__ == '__main__':
    unittest.main()
