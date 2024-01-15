// NOTE - Default allocator is twice as fast as jemalloc, at least in the only test case.
// TODO - Bench with more types
//      - Bench cache alignement (w/wo)
use {
    criterion::{criterion_group, criterion_main, Criterion},
    ring_buffer::RingBuffer,
    std::thread,
};

const VAL: &str = "BTC-USD";

fn ring_buffer(capacity: usize) {
    let (mut producer, consumer) = RingBuffer::new(capacity);
    let p = thread::spawn(move || {
        for _ in 0..capacity {
            _ = producer.push(VAL);
        }
    });
    let c = thread::spawn(move || for _ in consumer {});
    p.join().unwrap();
    c.join().unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
    // TODO - default / jemalloc
    c.bench_function("ring-buffer 4096", |b| b.iter(|| ring_buffer(4096)));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
