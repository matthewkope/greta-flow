import unittest
from punctuation import score


class PunctuationTests(unittest.TestCase):
    def test_same_sequence_wrong_position(self):
        self.assertEqual(score('First, ship it.', 'First ship, it.'), {'errors': 2, 'reference_marks': 2})

    def test_number_and_name_internal_marks_are_not_sentence_punctuation(self):
        text = 'Next.js costs $9,876.25 at 3:30, right?'
        self.assertEqual(score(text, text), {'errors': 0, 'reference_marks': 2})

    def test_number_format_normalization_preserves_boundaries(self):
        self.assertEqual(score('It costs $9,876.25, right?', 'It costs $9876.25, right?')['errors'], 0)

    def test_word_insertion_does_not_shift_every_later_boundary(self):
        self.assertEqual(score('We ship today, right?', 'We will ship today, right?')['errors'], 0)

    def test_missing_question_and_extra_mark(self):
        self.assertEqual(score('Ready?', 'Ready.'), {'errors': 1, 'reference_marks': 1})
        self.assertEqual(score('Ready', 'Ready!'), {'errors': 1, 'reference_marks': 0})

    def test_empty_transcript(self):
        self.assertEqual(score('Wait, stop.', ''), {'errors': 2, 'reference_marks': 2})


if __name__ == '__main__':
    unittest.main()
