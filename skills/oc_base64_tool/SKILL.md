---
name: base64_tool
description: Encodes or decodes text and data using Base64.
category: utility
parameters:
  type: object
  properties:
    input:
      type: string
      description: Text to encode or Base64 to decode
    mode:
      type: string
      description: encode or decode
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
