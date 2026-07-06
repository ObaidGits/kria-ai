---
name: sentiment_basic
description: Analyzes text and returns a basic positive, negative, or neutral score.
category: data
parameters:
  type: object
  properties:
    text:
      type: string
      description: Text to analyze
  required:
  - text
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
