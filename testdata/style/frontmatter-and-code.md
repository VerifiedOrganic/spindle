---
title: Introduction to Mana Loops
author: Archmage Eldrin
tags: [tutorial, magic]
---

# Introduction to Mana Loops

To construct a basic loop, a mage must program their core to cycle mana at a fixed frequency. The system uses a specific syntactic loop structure:

```rust
fn cycle_mana(pool: &mut ManaPool, rate: u32) {
    for _ in 0..rate {
        pool.circulate();
    }
}
```

<!-- Remember: too much cycle rate will burn the channels! -->

Ensure you do not exceed the channel capacity, or the results will be explosive.
