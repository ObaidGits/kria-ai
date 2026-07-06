---
name: slug_generator
description: Generates a URL-safe slug from an arbitrary title string.
category: utility
parameters:
  type: object
  properties:
    text:
      type: string
      description: Title or phrase to slugify
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
