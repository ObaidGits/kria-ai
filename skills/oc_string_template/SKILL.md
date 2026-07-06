---
name: string_template
description: Formats a template string by substituting named placeholder variables.
category: utility
parameters:
  type: object
  properties:
    template:
      type: string
      description: Template with {name} placeholders
    values:
      type: object
      description: Map of placeholder names to values
  required:
  - template
  - values
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
