---
"@logos-kit/client": minor
---

`decodeBlock` reads LEZ v0.3.1 blocks: their header no longer carries the producer key and signature, so `BlockHeader.producer` is gone.
