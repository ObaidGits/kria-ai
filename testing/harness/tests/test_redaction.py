from __future__ import annotations

import unittest

from testing.harness.reporting.redaction import redact_json, redact_text


class RedactionTests(unittest.TestCase):
    def test_redacts_secret_keys(self) -> None:
        value = redact_json({"api_key": "abc123456789", "nested": {"token": "secret-token"}})
        self.assertEqual(value["api_key"], "<redacted>")
        self.assertEqual(value["nested"]["token"], "<redacted>")

    def test_redacts_bearer_and_long_tokens(self) -> None:
        text = redact_text("Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456")
        self.assertIn("<redacted>", text)
        self.assertNotIn("abcdefghijklmnopqrstuvwxyz123456", text)


if __name__ == "__main__":
    unittest.main()
