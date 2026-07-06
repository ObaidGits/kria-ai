---
name: number_base_converter
description: Converts an integer between binary, octal, decimal, and hexadecimal.
category: utility
parameters:
  type: object
  properties:
    value:
      type: string
      description: The number as text
    from_base:
      type: integer
      description: Source base
    to_base:
      type: integer
      description: Target base
  required:
  - value
  - from_base
  - to_base
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
