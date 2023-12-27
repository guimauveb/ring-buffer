// TODO - Docstring

//! Lock-free ring buffer.
use std::{
    alloc::{self, Layout},
    marker::PhantomData,
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Clone, Debug)]
pub enum BufferError {
    BufferFull,
}

#[allow(dead_code)]
enum AllocInit {
    /// The contents of the new memory are uninitialized.
    Uninitialized,
    /// The new memory is guaranteed to be zeroed.
    Zeroed,
}

#[inline]
fn alloc_guard(alloc_size: usize) -> Result<(), ()> {
    if usize::BITS < 64 && alloc_size > isize::MAX as usize {
        //Err(CapacityOverflow.into())
        todo!("Handle alloc_guard error");
    } else {
        Ok(())
    }
}

/// A low-level utility for more ergonomically allocating, reallocating, and deallocating
/// a buffer of memory on the heap without having to worry about all the corner cases
/// involved. This type is excellent for building your own data structures like Vec and VecDeque.
struct RawVec<T> {
    ptr: NonNull<T>,
    cap: usize,
}

unsafe impl<T: Send> Send for RawVec<T> {}
unsafe impl<T: Sync> Sync for RawVec<T> {}

impl<T> RawVec<T> {
    // TODO - Call alloc error methods.
    fn allocate_in(capacity: usize, init: AllocInit) -> Self {
        let layout = match Layout::array::<T>(capacity) {
            Ok(layout) => layout,
            Err(_) => todo!("Handle alloc_guard error"), // capacity_overflow(),
        };
        match alloc_guard(layout.size()) {
            Ok(_) => {}
            Err(_) => todo!("Handle alloc_guard error"), // capacity_overflow(),
        }
        unsafe {
            let ptr = match init {
                AllocInit::Uninitialized => alloc::alloc(layout),
                AllocInit::Zeroed => alloc::alloc_zeroed(layout),
            };
            // Allocators currently return a `NonNull<[u8]>` whose length
            // matches the size requested. If that ever changes, the capacity
            // here should change to `ptr.len() / mem::size_of::<T>()`.
            Self {
                ptr: NonNull::new_unchecked(ptr.cast()),
                cap: capacity,
            }
        }
    }

    /// Constructs a new RawVec<T> with at least the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::allocate_in(capacity, AllocInit::Zeroed)
    }

    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            ptr: NonNull::dangling(),
            cap: 0,
        }
    }
}

impl<T> Drop for RawVec<T> {
    fn drop(&mut self) {
        if self.cap != 0 {
            let layout = Layout::array::<T>(self.cap).unwrap();
            unsafe {
                alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}

/// Thread safe pre-allocated contiguous buffer.
// TODO - Add a dropped field to know when the prod/cons has been dropped.
struct Buffer<T> {
    buffer: RawVec<Option<T>>,
    capacity: usize,
    // TODO - Cache alignement
    read: AtomicUsize,
    write: AtomicUsize,
}

unsafe impl<T: Send> Send for Buffer<T> {}
unsafe impl<T: Sync> Sync for Buffer<T> {}

impl<T> Buffer<T> {
    fn ptr(&self) -> *mut Option<T> {
        self.buffer.ptr.as_ptr()
    }

    /// Push a new element in the underlying buffer.
    ///
    /// Increments the `write` pointer by 1.
    pub fn write(&mut self, elem: T) -> Result<(), BufferError> {
        let write = self.write.load(Ordering::Acquire);
        let mut next_write = write + 1;
        if next_write == self.capacity {
            next_write = 0;
        }
        // Let the caller do whatever it wants until read has caught up.
        if next_write == self.read.load(Ordering::Acquire) {
            return Err(BufferError::BufferFull);
        }
        unsafe {
            ptr::write(&mut *self.ptr().add(write), Some(elem));
        }
        self.write.store(next_write, Ordering::Release);

        Ok(())
    }

    /// Remove the element at the given index.
    ///
    /// Increments the `read` pointer by 1.
    pub fn read(&mut self) -> Option<T> {
        let read = self.read.load(Ordering::Acquire);
        // Buffer is empty
        if read == self.write.load(Ordering::Acquire) {
            None
        } else {
            let mut next_read = read + 1;
            if next_read == self.capacity {
                next_read = 0;
            }
            unsafe {
                let elem = ptr::read(&mut *self.ptr().add(read));
                self.read.store(next_read, Ordering::Release);
                elem
            }
        }
    }
}

/// New type around a `Buffer` raw pointer so that `Send` and `Sync` can be derived.
struct BufferRaw<T>(*mut Buffer<T>);

unsafe impl<T: Send> Send for BufferRaw<T> {}
unsafe impl<T: Sync> Sync for BufferRaw<T> {}

impl<T> BufferRaw<T> {
    fn ptr(&self) -> *mut Buffer<T> {
        self.0
    }
}

/// A producer interface into a (Buffer)[Buffer]
pub struct Producer<T> {
    buffer: BufferRaw<T>,
}

/// A consumer interface into a (Buffer)[Buffer]
pub struct Consummer<T> {
    buffer: BufferRaw<T>,
}

/// Thread safe pre-allocated contiguous ring buffer.
pub struct RingBuffer<T> {
    _phantom: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    /// Initialize ring buffer with the given capacity.
    pub fn new(capacity: usize) -> (Producer<T>, Consummer<T>) {
        let buffer = Box::into_raw(Box::new(Buffer {
            buffer: RawVec::with_capacity(capacity),
            capacity,
            read: 0.into(),
            write: 0.into(),
        }));
        (
            Producer {
                buffer: BufferRaw(buffer),
            },
            Consummer {
                buffer: BufferRaw(buffer),
            },
        )
    }
}

impl<T> Producer<T> {
    // TODO - Implement Future for Buffer so that we can await instead of blocking (separate impl)
    //      - Check if consumer has been dropped
    pub fn send(&mut self, elem: T) -> Result<(), BufferError> {
        unsafe { self.buffer.ptr().as_mut().unwrap().write(elem) }
    }
}

// TODO - Check if producer has been dropped
impl<T> Consummer<T> {
    pub fn recv(&mut self) -> Option<T> {
        unsafe { self.buffer.ptr().as_mut().unwrap().read() }
    }
}

// TODO
impl<T> Iterator for Consummer<T> {
    type Item = Option<T>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.recv())
    }
}

#[cfg(test)]
mod tests {
    use {super::RingBuffer, minstant::Instant, std::thread};

    const BUFFER_SIZE: usize = 16;

    // TODO - Benchmark with TSC (with no logging)
    #[test]
    fn buffer() {
        let (mut producer, mut consumer) = RingBuffer::<String>::new(BUFFER_SIZE);

        let producer = thread::spawn(move || loop {
            for i in 0..BUFFER_SIZE * 64 {
                //let start = Instant::now();
                if let Err(_) = producer.send(i.to_string()) {
                    //println!("Buffer is full!");
                }
                //println!("Took {:?} to produce value", start.elapsed());
            }
        });
        let consumer = thread::spawn(move || {
            loop {
                //let start = Instant::now();
                match consumer.next() {
                    Some(Some(_)) => {
                        //println!("received {val:?}");
                    }
                    Some(None) => {
                        //println!("Buffer is empty");
                    }
                    _ => {}
                }
                //println!("Took {:?} to consume value", start.elapsed());
            }
        });
        producer.join().unwrap();
        consumer.join().unwrap();
    }
}