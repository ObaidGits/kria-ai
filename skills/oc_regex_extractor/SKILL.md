---
name: regex_extractor
description: Extracts all matches of a regular expression from a text.
category: utility
parameters:
  type: object
  properties:
    text:
      type: string
      description: Input text
    pattern:
      type: string
      description: Regular expression pattern
  required:
  - text
  - pattern
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
