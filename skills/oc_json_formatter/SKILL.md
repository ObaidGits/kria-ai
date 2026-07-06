---
name: json_formatter
description: Formats, minifies, and validates a JSON document.
category: utility
parameters:
  type: object
  properties:
    json:
      type: string
      description: JSON text
    mode:
      type: string
      description: pretty, minify, or validate
  required:
  - json
  - mode
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
