import json
import tempfile
import unittest
from pathlib import Path

from summarize import read, stats


class CompletenessTests(unittest.TestCase):
    def test_missing_and_duplicate_measurements_are_rejected(self):
        manifest = [{"id": "one"}, {"id": "two"}]
        rows = [{"event": "load"}]
        rows += [{"event": "offline_sink", "id": clip, "pass": p}
                 for clip, p in [("one", 0), ("one", 1), ("two", 1)]]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "run.jsonl"
            for records, valid in [(rows, True), (rows[:-1], False),
                                   (rows + [rows[-1]], False)]:
                path.write_text("\n".join(map(json.dumps, records)))
                if valid:
                    self.assertEqual(len(read(path, manifest, ["offline_sink"], 1)), 3)
                else:
                    with self.assertRaises(ValueError):
                        read(path, manifest, ["offline_sink"], 1)

    def test_nearest_rank_and_invalid_timings(self):
        self.assertEqual(stats(list(range(1, 21)))["p95_ms"], 19)
        for values in [[], [float("nan")], [-1]]:
            with self.assertRaises(ValueError):
                stats(values)


if __name__ == "__main__":
    unittest.main()
