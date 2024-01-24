# Lock-free, thread safe ring buffer.

Implementation of a lock-free, thread safe ring buffer in Rust.

## Notes
I made this project to learn more about lock-free datastructures, unsafe Rust and asynchronous code.

## Architecture
The ring buffer is implemented using two raw pointers, one for the producer and one for the consumer.
Because having two mutable references into the same block of memory is inherently unsafe, the implementation uses `unsafe` code (which is not fully tested yet).

### Memory
The consumer `Drop` implementation is responsible for the ring buffer memory deallocation.

### Behaviour
When the buffer is full, the current implementation of the `write` method will busy-spin until the consumer has caught up and memory has been freed. __The unread data
is not overwritten__. This is the fastest option (when not overwritting data) but not necessarily the best one depending on the code that uses the buffer. Two other "flavours" are being considered:
  - An implementation where threads are parked/unparked when the channel is empty or full.
  - An `async` implementation:
       - From the consumer side, when the buffer is empty, `await` for the producer until it produces a value or is dropped.
       - From the producer side, when the buffer is full, `await` for the consumer until it consumes a value or is dropped.

## TODO
  - Docstring

  ### Features
  - Async implementation for producer/consumer so that we can await instead of blocking:
     - From the consumer side, await for the producer to produce a value or be dropped
     - From the producer side, await for the consumer to read a value to free space or be dropped
  - Implementation where threads are parked/unparked when channel is empty/full

  ### Tests
  - Memory safety

  ### Benchmarks
  - Bench default allocator and jemalloc
  - Bench with more types
  - Bench w/wo cache alignment
  - Bench optimizations from https://rigtorp.se/ringbuffer/
  - Bench park/unpark threads
  - Bench async implementation

## Bonus
  - Compare perf with/without pinned threads
