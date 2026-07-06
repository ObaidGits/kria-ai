---
name: password_generator
description: Generates a cryptographically random password of a given length.
category: utility
parameters:
  type: object
  properties:
    length:
      type: integer
      description: Password length, 8-128
    symbols:
      type: boolean
      description: Include punctuation symbols
  required:
  - length
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
