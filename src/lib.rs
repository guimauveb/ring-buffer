// TODO - Docstring
//      - Benchmark with TSC (with no logging)
//      - Implement Future for Buffer so that we can await instead of blocking (separate impl)
//      - Compare perf with/without pinned threads
//      - Cache alignement between read/write to avoid false sharing (https://en.cppreference.com/w/cpp/thread/hardware_destructive_interference_size) and compare perf
//      with/without padding.

//! Lock-free ring buffer.
use std::{
    alloc::{self, Layout},
    marker::PhantomData,
    ptr::{self, NonNull},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[allow(dead_code)]
enum AllocInit {
    /// The contents of the new memory are uninitialized.
    Uninitialized,
    /// The new memory is guaranteed to be zeroed.
    Zeroed,
}

#[derive(Debug)]
enum AllocError {
    CapacityOverflow,
}

/// Ensure that the new allocation doesn't exceed `isize::MAX` bytes.
#[inline]
fn alloc_guard(alloc_size: usize) -> Result<(), AllocError> {
    if alloc_size > isize::MAX as usize {
        Err(AllocError::CapacityOverflow)
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BufferError {
    /// Buffer is full
    BufferFull,
    /// Consumer has been dropped
    Write,
    /// Producer has been dropped
    Read,
}

/// Thread safe pre-allocated contiguous buffer.
struct Buffer<T> {
    ptr: NonNull<T>,
    capacity: usize,
    // TODO - Cache alignement (https://en.cppreference.com/w/cpp/thread/hardware_destructive_interference_size)
    read: AtomicUsize,
    write: AtomicUsize,
    dropped: AtomicBool,
}

impl<T> Drop for Buffer<T> {
    fn drop(&mut self) {
        if self.capacity != 0 {
            let layout = Layout::array::<T>(self.capacity).unwrap();
            unsafe {
                alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}

unsafe impl<T: Send> Send for Buffer<T> {}
unsafe impl<T: Sync> Sync for Buffer<T> {}

impl<T> Buffer<T> {
    // TODO - Allocate capacity + padding to avoid false sharing
    fn allocate(capacity: usize, init: AllocInit) -> NonNull<T> {
        let layout = match Layout::array::<T>(capacity) {
            Ok(layout) => layout,
            Err(err) => panic!("Capacity overflow: {err:?}"),
        };
        match alloc_guard(layout.size()) {
            Ok(_) => {}
            Err(err) => panic!("Capacity overflow: {err:?}"),
        }
        unsafe {
            let ptr = match init {
                AllocInit::Uninitialized => alloc::alloc(layout),
                AllocInit::Zeroed => alloc::alloc_zeroed(layout),
            };
            // Allocators currently return a `NonNull<[u8]>` whose length
            // matches the size requested. If that ever changes, the capacity
            // here should change to `ptr.len() / mem::size_of::<T>()`.
            NonNull::new_unchecked(ptr.cast())
        }
    }

    /// Constructs a new [Buffer] with at the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ptr: Self::allocate(capacity, AllocInit::Zeroed),
            capacity,
            read: 0.into(),
            write: 0.into(),
            dropped: false.into(),
        }
    }

    /// Write a new element in the underlying buffer.
    ///
    /// If the consumer was dropped, [BufferError::Write] is returned and it should no longer be possible to consume values from the buffer.
    /// If the buffer is full, [BufferError::BufferFull] is returned. It's up to the caller to call this method again until space has been freed.
    fn write(&mut self, elem: T) -> Result<(), BufferError> {
        if self.dropped.load(Ordering::Acquire) {
            return Err(BufferError::Write);
        };
        let write = self.write.load(Ordering::Acquire);
        let mut next_write = write + 1;
        if next_write == self.capacity {
            next_write = 0;
        }
        // Return immediately Let the caller do whatever it wants until read has caught up.
        // TODO - In future impl, set the writer to awake the read caller.
        if next_write == self.read.load(Ordering::Acquire) {
            return Err(BufferError::BufferFull);
        }
        unsafe {
            ptr::write(&mut *self.ptr.as_ptr().add(write), elem);
        }
        self.write.store(next_write, Ordering::Release);

        Ok(())
    }

    /// Read the next readable element from the buffer.
    ///
    /// `None` is returned if the buffer is empty.
    fn read(&mut self) -> Result<Option<T>, BufferError> {
        let read = self.read.load(Ordering::Acquire);
        // Buffer is empty
        // TODO - If buffer is empty and producer was closed, return BufferError::Read and stop iterator.
        if read == self.write.load(Ordering::Acquire) {
            if self.dropped.load(Ordering::Acquire) {
                Err(BufferError::Read)
            } else {
                Ok(None)
            }
        } else {
            let mut next_read = read + 1;
            if next_read == self.capacity {
                next_read = 0;
            }
            unsafe {
                let elem = ptr::read(&*self.ptr.as_ptr().add(read));
                self.read.store(next_read, Ordering::Release);
                Ok(Some(elem))
            }
        }
    }
}

/// New type wrapping a [Buffer] raw pointer so that [Send] and [Sync] can be derived.
struct BufferRaw<T>(*mut Buffer<T>);

unsafe impl<T: Send> Send for BufferRaw<T> {}
unsafe impl<T: Sync> Sync for BufferRaw<T> {}

impl<T> BufferRaw<T> {
    /// Return the underlying raw pointer.
    fn ptr(&self) -> *mut Buffer<T> {
        self.0
    }
}

/// A producer interface into a [Buffer].
pub struct Producer<T> {
    buffer: BufferRaw<T>,
}

impl<T> Producer<T> {
    /// Push a new element to the underlying buffer.
    ///
    /// If the consumer was dropped, [BufferError::Write] is returned and it should no longer be possible to consume values from the buffer.
    /// If the buffer is full, [BufferError::BufferFull] is returned. It's up to the caller to call this method again until space has been freed.
    #[inline]
    pub fn push(&mut self, elem: T) -> Result<(), BufferError> {
        unsafe { self.buffer.ptr().as_mut().unwrap().write(elem) }
    }
}

impl<T> Drop for Producer<T> {
    // TODO - Need to manually free memory?
    fn drop(&mut self) {
        unsafe { (*self.buffer.ptr()).dropped.store(true, Ordering::Release) };
    }
}

/// A consumer interface into a [Buffer]
pub struct Consumer<T> {
    buffer: BufferRaw<T>,
}

impl<T> Consumer<T> {
    /// Pop the next available element from the buffer.
    ///
    /// `None` is returned if the buffer is empty.
    #[inline]
    pub fn pop(&mut self) -> Result<Option<T>, BufferError> {
        unsafe { self.buffer.ptr().as_mut().unwrap().read() }
    }
}

impl<T> Drop for Consumer<T> {
    // TODO - Need to manually free memory?
    fn drop(&mut self) {
        unsafe { (*self.buffer.ptr()).dropped.store(true, Ordering::Release) };
    }
}

impl<T> Iterator for Consumer<T> {
    type Item = Option<T>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            match self.buffer.ptr().as_mut().unwrap().read() {
                Ok(Some(elem)) => Some(Some(elem)),
                Ok(None) => Some(None),
                Err(err) => {
                    if err == BufferError::Read {
                        None
                    } else {
                        Some(None)
                    }
                }
            }
        }
    }
}

/// Thread safe pre-allocated contiguous ring buffer.
pub struct RingBuffer<T> {
    _phantom: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    /// Initialize a ring buffer with the given capacity.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(capacity: usize) -> (Producer<T>, Consumer<T>) {
        let buffer = Box::into_raw(Box::new(Buffer::with_capacity(capacity)));
        (
            Producer {
                buffer: BufferRaw(buffer),
            },
            Consumer {
                buffer: BufferRaw(buffer),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use {
        super::RingBuffer,
        minstant::Instant,
        std::{thread, time::Duration},
    };

    const BUFFER_SIZE: usize = 16;

    #[test]
    fn buffer() {
        let (mut producer, mut consumer) = RingBuffer::<String>::new(BUFFER_SIZE);

        let producer = thread::spawn(move || {
            for i in 0..BUFFER_SIZE * 64 {
                let start = Instant::now();
                if producer.push(i.to_string()).is_err() {
                    //println!("Buffer is full!");
                }
                println!("Took {:?} to produce value", start.elapsed());
            }
            thread::sleep(Duration::from_secs(10));
        });
        let consumer = thread::spawn(move || {
            loop {
                let start = Instant::now();
                match consumer.next() {
                    Some(Some(val)) => {
                        //println!("received {val:?}");
                    }
                    Some(None) => {
                        //println!("Buffer is empty");
                    }
                    None => {
                        println!("Channel is closed");
                        panic!();
                    }
                }
                println!("Took {:?} to consume value", start.elapsed());
            }
        });
        producer.join().unwrap();
        consumer.join().unwrap();
    }
}
