---
name: word_counter
description: Calculates the word, character, sentence, and line counts of a text.
category: utility
parameters:
  type: object
  properties:
    text:
      type: string
      description: Input text to analyze
  required:
  - text
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
