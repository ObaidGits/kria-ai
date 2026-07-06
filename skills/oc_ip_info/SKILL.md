---
name: ip_info
description: Fetches geolocation and network metadata for an IP address.
category: web
parameters:
  type: object
  properties:
    ip:
      type: string
      description: IPv4 or IPv6 address
  required:
  - ip
capabilities:
  network: true
  filesystem_read: false
  filesystem_write: false
  subprocess: false
  network_domains:
  - ip-api.com
  - ipinfo.io
resource_class: medium
timeout_secs: 30
---
