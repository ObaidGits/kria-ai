---
name: hash_generator
description: Calculates md5, sha1, sha256, or sha512 digests of an input string.
category: utility
parameters:
  type: object
  properties:
    input:
      type: string
      description: Text to hash
    algorithm:
      type: string
      description: md5, sha1, sha256, or sha512
  required:
  - input
  - algorithm
capabilities:
  network: false
  filesystem_read: false
  filesystem_write: false
  subprocess: false
resource_class: light
timeout_secs: 30
---
