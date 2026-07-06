---
name: uuid_generator
description: Generates random UUID (v4) or time-ordered (v7) identifiers.
category: utility
parameters:
  type: object
  properties:
    version:
      type: string
      description: 4 or 7
    count:
      type: integer
      description: How many to generate, default 1
  required: []
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
