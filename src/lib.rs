//! Lock-free, thread safe ring buffer.
#[cfg(feature = "jemalloc")]
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(feature = "jemalloc")]
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

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
#[repr(align(64))]
struct Buffer<T> {
    ptr: NonNull<T>,
    capacity: usize,
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
    fn allocate(capacity: usize, init: AllocInit) -> NonNull<T> {
        let layout = match Layout::array::<T>(capacity) {
            Ok(layout) => layout,
            Err(err) => {
                panic!("Capacity overflow: {err:?}");
            }
        };
        match alloc_guard(layout.size()) {
            Ok(_) => {}
            Err(err) => {
                panic!("Capacity overflow: {err:?}");
            }
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
        // Consumer was dropped, the channel is closed.
        if self.dropped.load(Ordering::Acquire) {
            return Err(BufferError::Write);
        }
        let write = self.write.load(Ordering::Acquire);
        let mut next_write = write + 1;
        if next_write == self.capacity {
            next_write = 0;
        }
        loop {
            let read = self.read.load(Ordering::Acquire);
            // Buffer is full. Busy spin until read has caught up.
            if next_write == read {
                continue;
            }
            break;
        }
        unsafe {
            ptr::write(&mut *self.ptr.as_ptr().add(write), elem);
        }
        self.write.store(next_write, Ordering::Release);

        Ok(())
    }

    /// Read the next readable element from the buffer.
    ///
    /// If the producer was dropped and the buffer is empty, [BufferError::Read] is returned to
    /// signal that the buffer will not contain any value anymore.
    /// `None` is returned if the buffer is empty but the producer is still alive.
    fn read(&mut self) -> Result<Option<T>, BufferError> {
        let read = self.read.load(Ordering::Acquire);
        // Buffer is empty
        if read == self.write.load(Ordering::Acquire) {
            // Buffer is empty and producer was dropped, the channel is closed.
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
    /// Only marks the producer as dropped so that the consumer knows that this channel is closed
    /// and will no longer receive values.
    ///
    /// It is the [Consumer] responsibility to free the buffer in its drop implementation.
    fn drop(&mut self) {
        unsafe {
            let buffer = self.buffer.ptr();
            (*buffer).dropped.store(true, Ordering::Release);
        }
    }
}

/// A consumer interface into a [Buffer].
pub struct Consumer<T> {
    buffer: BufferRaw<T>,
}

impl<T> Consumer<T> {
    /// Pop the next available element from the buffer.
    ///
    /// If the producer was dropped and the buffer is empty, [BufferError::Read] is returned to
    /// signal that the buffer will not contain any value anymore.
    /// `None` is returned if the buffer is empty but the producer is still alive.
    #[inline]
    pub fn pop(&mut self) -> Result<Option<T>, BufferError> {
        unsafe { self.buffer.ptr().as_mut().unwrap().read() }
    }
}

impl<T> Drop for Consumer<T> {
    fn drop(&mut self) {
        unsafe {
            let buffer = self.buffer.ptr();
            // Mark the consumer as dropped so that the producer knows that this channel is closed
            // and will no longer receive values.
            (*buffer).dropped.store(true, Ordering::Release);
            // Free the buffer memory.
            ptr::drop_in_place(buffer);
        }
    }
}

impl<T> Iterator for Consumer<T> {
    type Item = Option<T>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            match self.buffer.ptr().as_mut().unwrap().read() {
                Ok(Some(elem)) => Some(Some(elem)),
                Ok(None) => Some(None),
                Err(_) => {
                    // Buffer is empty and producer was dropped, stop the iteration.
                    None
                }
            }
        }
    }
}

/// Lock-free, thread safe ring buffer.
pub struct RingBuffer<T> {
    _phantom: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    /// Initialize a ring buffer with the given capacity.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(capacity: usize) -> (Producer<T>, Consumer<T>) {
        let buffer = Box::into_raw(Box::new(Buffer::with_capacity(capacity + 1)));
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
    use {super::RingBuffer, std::thread};

    const BUFFER_SIZE: usize = 16;

    #[test]
    fn ring_buffer() {
        let (mut producer, consumer) = RingBuffer::<String>::new(BUFFER_SIZE);
        let p = thread::spawn(move || {
            for i in 0..BUFFER_SIZE * 1000 {
                _ = producer.push(i.to_string());
            }
        });
        let c = thread::spawn(move || for _ in consumer {});
        p.join().unwrap();
        c.join().unwrap();
    }
}
