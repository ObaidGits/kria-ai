---
name: sql_formatter
description: Formats a SQL statement with consistent indentation and keyword casing.
category: developer
parameters:
  type: object
  properties:
    sql:
      type: string
      description: SQL statement to format
  required:
  - sql
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: medium
timeout_secs: 30
---
