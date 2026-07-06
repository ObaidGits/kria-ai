---
name: csv_to_json
description: Parses CSV text and converts it to a JSON array of row objects.
category: data
parameters:
  type: object
  properties:
    csv:
      type: string
      description: CSV text with a header row
    delimiter:
      type: string
      description: Optional column delimiter, default comma
  required:
  - csv
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
