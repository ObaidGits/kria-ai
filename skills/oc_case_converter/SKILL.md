---
name: case_converter
description: Converts text between upper, lower, title, snake, kebab, and camel case.
category: utility
parameters:
  type: object
  properties:
    text:
      type: string
      description: Input text
    mode:
      type: string
      description: 'One of: upper, lower, title, snake, kebab, camel'
  required:
  - text
  - mode
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
