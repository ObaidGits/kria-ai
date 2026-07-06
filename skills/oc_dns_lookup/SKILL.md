---
name: dns_lookup
description: Resolves DNS records (A, AAAA, MX, TXT) for a hostname.
category: web
parameters:
  type: object
  properties:
    hostname:
      type: string
      description: Domain name to resolve
    record:
      type: string
      description: 'Record type: A, AAAA, MX, or TXT'
  required:
  - hostname
capabilities:
  network: true
  filesystem_read: false
  filesystem_write: false
  subprocess: false
  network_domains:
  - dns.google
  - cloudflare-dns.com
resource_class: medium
timeout_secs: 30
---
