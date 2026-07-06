---
name: code_sandbox
description: Executes Python 3 code in an isolated container and returns stdout and
  stderr.
category: developer
parameters:
  type: object
  properties:
    code:
      type: string
      description: Python 3 source code to execute
  required:
  - code
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: true
resource_class: medium
timeout_secs: 30
---
