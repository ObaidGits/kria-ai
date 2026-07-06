---
name: yaml_to_json
description: Converts a YAML document into equivalent JSON.
category: data
parameters:
  type: object
  properties:
    yaml:
      type: string
      description: YAML text
  required:
  - yaml
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
