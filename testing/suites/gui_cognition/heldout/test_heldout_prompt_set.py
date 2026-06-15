"""Guard tests for the frozen GUI Cognition held-out prompt set (Task 0.1).

These protect the invariants required by Requirements 17, 18, 20:
* >= 5 prompts per family, all 21 capability families present (R17).
* The set is FROZEN: the committed digest lock must match the prompts, so the
  set cannot be silently edited to make a build pass (R17/R18).
* Loader + verifier behave correctly and detect tampering (R20 isolation hinges
  on a stable, non-mutable scoring set).

Run from repo root:
    python3 -m pytest testing/suites/gui_cognition/heldout/test_heldout_prompt_set.py
"""
from __future__ import annotations

import copy
import json
import unittest

from testing.tools.heldout_prompt_set import (
    EXPECTED_FAMILIES,
    EXPECTED_FAMILY_COUNT,
    LOCK_PATH,
    MIN_PROMPTS_PER_FAMILY,
    SET_PATH,
    VALID_KINDS,
    _load_raw,
    check_invariants,
    compute_digest,
    family_counts,
    load_prompts,
    verify_frozen,
)


class HeldoutPromptSetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.data = _load_raw()

    def test_set_and_lock_files_exist(self) -> None:
        self.assertTrue(SET_PATH.exists(), SET_PATH)
        self.assertTrue(LOCK_PATH.exists(), LOCK_PATH)

    def test_all_21_families_present(self) -> None:
        caps = [fam["cap"] for fam in self.data["families"]]
        self.assertEqual(len(caps), EXPECTED_FAMILY_COUNT)
        self.assertEqual(sorted(caps), sorted(EXPECTED_FAMILIES))

    def test_minimum_five_prompts_per_family(self) -> None:
        for cap, n in family_counts().items():
            self.assertGreaterEqual(
                n, MIN_PROMPTS_PER_FAMILY, f"{cap} has only {n} prompts"
            )

    def test_kinds_are_valid(self) -> None:
        for fam in self.data["families"]:
            self.assertIn(fam["kind"], VALID_KINDS, fam["cap"])

    def test_ambiguity_and_boundary_kinds_are_correct(self) -> None:
        by_cap = {fam["cap"]: fam["kind"] for fam in self.data["families"]}
        self.assertEqual(by_cap["C18_ambiguity"], "ask")
        self.assertEqual(by_cap["C19_boundary"], "boundary")

    def test_no_duplicate_prompts_anywhere(self) -> None:
        prompts = [p.text for p in load_prompts()]
        self.assertEqual(len(prompts), len(set(prompts)), "duplicate prompt text found")

    def test_invariants_pass_on_committed_set(self) -> None:
        self.assertEqual(check_invariants(self.data), [])

    def test_committed_set_is_frozen_and_valid(self) -> None:
        # The committed lock digest MUST match the committed prompts.
        self.assertEqual(verify_frozen(), [])

    def test_loader_returns_flat_prompt_list(self) -> None:
        prompts = load_prompts()
        self.assertEqual(len(prompts), sum(family_counts().values()))
        self.assertTrue(all(p.text and p.cap and p.kind for p in prompts))

    def test_tampering_with_a_prompt_breaks_the_freeze(self) -> None:
        # Property: ANY change to a scored prompt changes the digest, so an
        # edit made "to pass a build" is detected.
        original_digest = compute_digest(self.data)
        tampered = copy.deepcopy(self.data)
        tampered["families"][0]["prompts"][0] = (
            tampered["families"][0]["prompts"][0] + " (edited)"
        )
        self.assertNotEqual(compute_digest(tampered), original_digest)

    def test_cosmetic_description_edit_does_not_change_digest(self) -> None:
        # Only scored prompt content is hashed; descriptions/policy are not.
        original_digest = compute_digest(self.data)
        cosmetic = copy.deepcopy(self.data)
        cosmetic["description"] = "totally different description text"
        self.assertEqual(compute_digest(cosmetic), original_digest)

    def test_lock_records_match_actual_counts(self) -> None:
        lock = json.loads(LOCK_PATH.read_text(encoding="utf-8"))
        self.assertEqual(lock["family_count"], EXPECTED_FAMILY_COUNT)
        self.assertEqual(
            lock["total_prompts"], sum(family_counts().values())
        )
        self.assertEqual(lock["digest"], compute_digest(self.data))


if __name__ == "__main__":
    unittest.main()
