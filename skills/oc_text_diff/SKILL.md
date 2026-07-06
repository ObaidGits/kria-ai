---
name: text_diff
description: Generates a unified diff between two text inputs.
category: developer
parameters:
  type: object
  properties:
    left:
      type: string
      description: Original text
    right:
      type: string
      description: Changed text
  required:
  - left
  - right
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: medium
timeout_secs: 30
---
