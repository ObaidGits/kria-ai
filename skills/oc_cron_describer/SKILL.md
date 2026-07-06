---
name: cron_describer
description: Converts a cron expression into a human-readable schedule description.
category: developer
parameters:
  type: object
  properties:
    expression:
      type: string
      description: A 5-field cron expression
  required:
  - expression
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: medium
timeout_secs: 30
---
