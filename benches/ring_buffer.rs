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
            _ = producer.push_or_spin(VAL);
        }
    });
    let c = thread::spawn(move || for _ in consumer {});
    p.join().unwrap();
    c.join().unwrap();
}

fn ring_buffer_exceeds(capacity: usize) {
    let (mut producer, consumer) = RingBuffer::new(capacity);
    let p = thread::spawn(move || {
        for _ in 0..(capacity * capacity) {
            _ = producer.push_or_spin(VAL);
        }
    });
    let c = thread::spawn(move || for _ in consumer {});
    p.join().unwrap();
    c.join().unwrap();
}

fn ring_buffer_exceeds_pinned(capacity: usize) {
    let (mut producer, consumer) = RingBuffer::new(capacity);
    let p = thread::spawn(move || {
        core_affinity::set_for_current(core_affinity::CoreId { id: 1 });
        for _ in 0..capacity {
            _ = producer.push_or_spin(VAL);
        }
    });
    let c = thread::spawn(move || {
        core_affinity::set_for_current(core_affinity::CoreId { id: 2 });
        for _ in consumer {}
    });

    p.join().unwrap();
    c.join().unwrap();
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("ring-buffer 4096 str", |b| b.iter(|| ring_buffer(4096)));
    c.bench_function("ring-buffer 4096 str exceeds capacity", |b| {
        b.iter(|| ring_buffer_exceeds(4096))
    });
    c.bench_function("ring-buffer 4096 str exceeds capacity pinned", |b| {
        b.iter(|| ring_buffer_exceeds_pinned(4096))
    });
    c.bench_function("ring-buffer 4096*4096 str exceeds capacity pinned", |b| {
        b.iter(|| ring_buffer_exceeds_pinned(4096 * 4096))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
