---
name: unit_converter
description: Converts a value between units of length, weight, or temperature.
category: productivity
parameters:
  type: object
  properties:
    value:
      type: number
      description: The numeric value
    from_unit:
      type: string
      description: Source unit
    to_unit:
      type: string
      description: Target unit
  required:
  - value
  - from_unit
  - to_unit
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
