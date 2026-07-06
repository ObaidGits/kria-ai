---
name: timestamp_converter
description: Converts between Unix epoch seconds and ISO-8601 date-time strings.
category: utility
parameters:
  type: object
  properties:
    input:
      type: string
      description: Epoch seconds or ISO-8601 string
    mode:
      type: string
      description: to_iso or to_epoch
  required:
  - input
  - mode
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
