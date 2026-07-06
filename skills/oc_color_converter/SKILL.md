---
name: color_converter
description: Converts colors between HEX, RGB, and HSL representations.
category: utility
parameters:
  type: object
  properties:
    input:
      type: string
      description: 'Color value, e.g. #1a2b3c or rgb(1,2,3)'
    target:
      type: string
      description: hex, rgb, or hsl
  required:
  - input
  - target
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
