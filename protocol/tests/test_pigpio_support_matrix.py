"""Structural checks for the machine-readable pigpiod support matrix."""

import json
import pathlib
import unittest


MATRIX_PATH = pathlib.Path(__file__).resolve().parents[1] / "pigpio-command-support.json"


class PigpioSupportMatrixTest(unittest.TestCase):
    def test_matrix_has_unique_commands_and_evidence_for_conformance(self):
        matrix = json.loads(MATRIX_PATH.read_text(encoding="utf-8"))
        allowed = set(matrix["statuses"])
        names = set()
        numbers = set()

        self.assertEqual(matrix["schema_version"], 1)
        self.assertEqual(matrix["compatibility_target"], "pigpiod")
        self.assertEqual(matrix["compatibility_claim"], "none")
        for command in matrix["commands"]:
            self.assertIn(command["status"], allowed)
            self.assertNotIn(command["name"], names)
            self.assertNotIn(command["number"], numbers)
            names.add(command["name"])
            numbers.add(command["number"])
            if command["status"] == "conformant":
                self.assertIsInstance(command["test"], str)
                self.assertTrue(command["test"])


if __name__ == "__main__":
    unittest.main()
