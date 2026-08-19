#!/usr/bin/env python3
"""Check the router's volume/brightness patterns against real user phrasings.

The patterns are copied verbatim from crates/kria-core/src/agent/router.rs so the
question "would the router have claimed this sentence?" is answered by the actual
expressions rather than by reading them.
"""
import re

VOLUME = [
    r"(?i)\b(volume|sound)\s*(set|to|at)\s*(\d+)\b",
    r"(?i)\b(set|change|put|increase|decrease|raise|lower|turn\s+up|turn\s+down)\b.{0,20}\b(volume|sound|speaker)\b",
    r"(?i)\b(turn|set|put|bring|crank|bump)\b.{0,20}\b(volume|sound)\b.{0,20}\b(up|down|to|at|higher|lower|louder|quieter)\b",
    r"(?i)\b(volume|sound)\s+(up|down|louder|quieter|higher|lower)\b",
    r"(?i)\b(volume|sound|speaker|awaaz)\s+(ko|set|badhao|ghataao|ghatao|badha|ghata|barhao|badhaao)\b|\b(volume|sound|speaker|awaaz)\s+\d+",
]

BRIGHTNESS = [
    r"(?i)\b(brightness)\s*(set|to|at)\s*(\d+)\b",
    r"(?i)\b(set|change|increase|decrease|raise|lower|turn\s+up|turn\s+down)\b.{0,20}\bbrightness\b",
    r"(?i)\b(turn|set|put|bring)\b.{0,20}\bbrightness\b.{0,20}\b(up|down|to|at|higher|lower|dimmer|brighter)\b",
    r"(?i)\bbrightness\s+(up|down|higher|lower|dimmer|brighter)\b",
    r"(?i)\bbrightness\s+(ko|set|badhao|ghataao|ghatao|badha|ghata|barhao|badhaao)\b|\bbrightness\s+\d+",
]

CASES = [
    # (sentence, which pattern group, what the user plainly meant)
    ("Turn Volume up to 40%", VOLUME, "set volume to 40"),
    ("Turn Volume Down to 70%", VOLUME, "set volume to 70"),
    ("turn the volume up", VOLUME, "raise volume"),
    ("turn volume down", VOLUME, "lower volume"),
    ("set volume to 40%", VOLUME, "set volume to 40"),
    ("turn up the volume", VOLUME, "raise volume"),
    ("volume 40", VOLUME, "set volume to 40"),
    ("increase the volume", VOLUME, "raise volume"),
    ("Turn brightness up to 60%", BRIGHTNESS, "set brightness 60"),
    ("turn brightness down", BRIGHTNESS, "lower brightness"),
    ("turn down the brightness", BRIGHTNESS, "lower brightness"),
    ("set brightness to 60", BRIGHTNESS, "set brightness 60"),
]


def main() -> None:
    width = max(len(c[0]) for c in CASES) + 2
    misses = 0
    for sentence, patterns, meaning in CASES:
        hit = next(
            (i for i, p in enumerate(patterns) if re.search(p, sentence)), None
        )
        if hit is None:
            misses += 1
            print(f"MISS  {sentence:<{width}} (meant: {meaning})")
        else:
            print(f"match {sentence:<{width}} -> pattern #{hit}")
    print(f"\n{misses} of {len(CASES)} phrasings are NOT routed to an OS tool.")


if __name__ == "__main__":
    main()
