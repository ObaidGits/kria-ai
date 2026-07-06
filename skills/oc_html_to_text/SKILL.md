---
name: html_to_text
description: Extracts readable plain text from an HTML document.
category: utility
parameters:
  type: object
  properties:
    html:
      type: string
      description: HTML source
  required:
  - html
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
