---
name: lorem_ipsum
description: Generates placeholder lorem ipsum words, sentences, or paragraphs.
category: utility
parameters:
  type: object
  properties:
    unit:
      type: string
      description: words, sentences, or paragraphs
    count:
      type: integer
      description: How many units
  required:
  - unit
  - count
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
