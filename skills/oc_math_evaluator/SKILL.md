---
name: math_evaluator
description: Calculates the result of a numeric arithmetic expression.
category: productivity
parameters:
  type: object
  properties:
    expression:
      type: string
      description: Arithmetic expression, e.g. 3*(4+5)
  required:
  - expression
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
