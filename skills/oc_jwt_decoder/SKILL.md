---
name: jwt_decoder
description: Decodes the header and payload of a JWT without verifying its signature.
category: developer
parameters:
  type: object
  properties:
    token:
      type: string
      description: The JWT string
  required:
  - token
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: medium
timeout_secs: 30
---
