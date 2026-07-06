---
name: http_get
description: Fetches the response body of an HTTP GET request for a URL.
category: web
parameters:
  type: object
  properties:
    url:
      type: string
      description: The absolute HTTPS URL to fetch
  required:
  - url
capabilities:
  network: true
  filesystem_read: false
  filesystem_write: false
  subprocess: false
  network_domains:
  - '*'
resource_class: medium
timeout_secs: 30
---
