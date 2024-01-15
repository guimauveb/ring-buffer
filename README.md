# Lock-free, thread safe ring buffer.

Implementation of a lock-free, thread safe ring buffer in Rust.

## TODO
  ### Features
  - Async implementation for producer/consumer so that we can await instead of blocking:
     - From the consumer side, await for the producer to produce a value or be dropped
     - From the producer side, await for the consumer to read a value to free space or be dropped
  - Implementation where threads are parked/unparked when channel is empty/full

  ### Tests 
  - Memory safety, deallocation

  ### Benchmarks 
  - Bench default allocator and jemalloc
  - Bench with more types
  - Bench w/wo cache alignment

## Bonus
  - Compare perf with/without pinned threads
