# Lock-free, thread safe ring buffer.

Implementation of a lock-free, thread safe ring buffer in Rust.

## TODO
  - Cache alignement between read/write to avoid false sharing (https://en.cppreference.com/w/cpp/thread/hardware_destructive_interference_size)
  - Compare perf with/without padding
  - Benchmark
  - Implement Future for producer/consumer so that we can await instead of blocking:
      - From the consumer side, await for the producer to produce a value or be dropped
      - From the producer side, await for the consumer to read a value to free space or be dropped
  - Tests

## Bonus
  - Compare perf with/without pinned threads
