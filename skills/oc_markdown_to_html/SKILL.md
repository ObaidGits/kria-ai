---
name: markdown_to_html
description: Converts a Markdown document into HTML.
category: utility
parameters:
  type: object
  properties:
    markdown:
      type: string
      description: Markdown source
  required:
  - markdown
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
