---
name: url_codec
description: Encodes or decodes URL percent-encoding for a string.
category: utility
parameters:
  type: object
  properties:
    input:
      type: string
      description: Text or encoded URL component
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
