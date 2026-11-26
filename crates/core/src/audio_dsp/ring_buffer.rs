//! Lock-free, cache-line-aligned Single-Producer Single-Consumer (SPSC) ring buffer.
//!
//! Used to decouple audio decoding/network fetching from real-time audio playback so
//! UI/network stalls never cause audio callback underruns. Zero allocations after creation.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Cache-line padded atomic index to prevent false sharing between producer & consumer threads.
#[repr(align(64))]
struct CachePaddedAtomic(AtomicUsize);

struct RingStorage<T> {
    buffer: Box<[UnsafeCell<MaybeUninit<T>>]>,
    mask: usize,
    head: CachePaddedAtomic,
    tail: CachePaddedAtomic,
}

// SAFETY: Single producer exclusively writes slots at `head` before Release-storing `head`,
// and single consumer exclusively reads slots at `tail` after Acquire-loading `head`.
unsafe impl<T: Send> Send for RingStorage<T> {}
unsafe impl<T: Send> Sync for RingStorage<T> {}

impl<T> Drop for RingStorage<T> {
    fn drop(&mut self) {
        let mut tail = self.tail.0.load(Ordering::Relaxed);
        let head = self.head.0.load(Ordering::Relaxed);
        while tail != head {
            let idx = tail & self.mask;
            // SAFETY: Every slot in `[tail, head)` is initialized.
            unsafe {
                (*self.buffer[idx].get()).assume_init_drop();
            }
            tail = tail.wrapping_add(1);
        }
    }
}

/// Lock-free SPSC ring buffer factory.
pub struct SpscRingBuffer;

impl SpscRingBuffer {
    /// Allocates a lock-free SPSC ring buffer rounded up to the next power-of-two capacity.
    pub fn with_capacity<T: Send>(min_capacity: usize) -> (SpscProducer<T>, SpscConsumer<T>) {
        let cap = min_capacity.max(2).next_power_of_two();
        let mut vec = Vec::with_capacity(cap);
        for _ in 0..cap {
            vec.push(UnsafeCell::new(MaybeUninit::uninit()));
        }
        let storage = Arc::new(RingStorage {
            buffer: vec.into_boxed_slice(),
            mask: cap - 1,
            head: CachePaddedAtomic(AtomicUsize::new(0)),
            tail: CachePaddedAtomic(AtomicUsize::new(0)),
        });
        (
            SpscProducer {
                storage: storage.clone(),
            },
            SpscConsumer { storage },
        )
    }
}

pub struct SpscProducer<T: Send> {
    storage: Arc<RingStorage<T>>,
}

pub struct SpscConsumer<T: Send> {
    storage: Arc<RingStorage<T>>,
}

impl<T: Send> SpscProducer<T> {
    pub fn capacity(&self) -> usize {
        self.storage.mask + 1
    }

    pub fn free_slots(&self) -> usize {
        let head = self.storage.head.0.load(Ordering::Relaxed);
        let tail = self.storage.tail.0.load(Ordering::Acquire);
        self.capacity().saturating_sub(head.wrapping_sub(tail))
    }

    pub fn push(&mut self, value: T) -> Result<(), T> {
        let head = self.storage.head.0.load(Ordering::Relaxed);
        let tail = self.storage.tail.0.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= self.capacity() {
            return Err(value);
        }
        let idx = head & self.storage.mask;
        // SAFETY: Slot at `head` is not readable by consumer until `head` is incremented with Release.
        unsafe {
            (*self.storage.buffer[idx].get()).write(value);
        }
        self.storage
            .head
            .0
            .store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }
}

impl<T: Copy + Send> SpscProducer<T> {
    /// Pushes as many elements from `slice` as fit without blocking; returns count pushed.
    pub fn push_slice(&mut self, slice: &[T]) -> usize {
        let head = self.storage.head.0.load(Ordering::Relaxed);
        let tail = self.storage.tail.0.load(Ordering::Acquire);
        let avail = self.capacity().saturating_sub(head.wrapping_sub(tail));
        let count = slice.len().min(avail);
        for (i, &val) in slice.iter().take(count).enumerate() {
            let idx = head.wrapping_add(i) & self.storage.mask;
            // SAFETY: Slots `[head, head + count)` are exclusively owned by the producer.
            unsafe {
                (*self.storage.buffer[idx].get()).write(val);
            }
        }
        if count > 0 {
            self.storage
                .head
                .0
                .store(head.wrapping_add(count), Ordering::Release);
        }
        count
    }
}

impl<T: Send> SpscConsumer<T> {
    pub fn capacity(&self) -> usize {
        self.storage.mask + 1
    }

    pub fn available(&self) -> usize {
        let head = self.storage.head.0.load(Ordering::Acquire);
        let tail = self.storage.tail.0.load(Ordering::Relaxed);
        head.wrapping_sub(tail)
    }

    pub fn pop(&mut self) -> Option<T> {
        let tail = self.storage.tail.0.load(Ordering::Relaxed);
        let head = self.storage.head.0.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let idx = tail & self.storage.mask;
        // SAFETY: Producer wrote this slot before Release-storing `head`.
        let val = unsafe { (*self.storage.buffer[idx].get()).assume_init_read() };
        self.storage
            .tail
            .0
            .store(tail.wrapping_add(1), Ordering::Release);
        Some(val)
    }
}

impl<T: Copy + Send> SpscConsumer<T> {
    /// Pops up to `out.len()` elements without blocking; returns count read.
    pub fn pop_slice(&mut self, out: &mut [T]) -> usize {
        let tail = self.storage.tail.0.load(Ordering::Relaxed);
        let head = self.storage.head.0.load(Ordering::Acquire);
        let avail = head.wrapping_sub(tail);
        let count = out.len().min(avail);
        for (i, dst) in out.iter_mut().take(count).enumerate() {
            let idx = tail.wrapping_add(i) & self.storage.mask;
            // SAFETY: Slots `[tail, tail + count)` were initialized before Release-storing `head`.
            *dst = unsafe { (*self.storage.buffer[idx].get()).assume_init_read() };
        }
        if count > 0 {
            self.storage
                .tail
                .0
                .store(tail.wrapping_add(count), Ordering::Release);
        }
        count
    }
}

impl SpscConsumer<f32> {
    /// Pops up to `out.len()` f32 samples; if an underrun occurs mid-buffer, applies a
    /// smooth up-to-64-frame fade-to-zero on the tail of popped samples and zeroes the rest
    /// so hardware callbacks never emit square-wave clicks.
    pub fn pop_with_underrun_fade(&mut self, out: &mut [f32], channels: usize) -> usize {
        let popped = self.pop_slice(out);
        if popped < out.len() {
            let ch = channels.max(1);
            let popped_frames = popped / ch;
            let fade_frames = popped_frames.min(64);
            if fade_frames > 0 {
                let start_frame = popped_frames - fade_frames;
                let denom = fade_frames as f32;
                for f in 0..fade_frames {
                    let gain = 1.0 - ((f + 1) as f32 / denom);
                    let base = (start_frame + f) * ch;
                    for c in 0..ch {
                        if base + c < popped {
                            out[base + c] *= gain;
                        }
                    }
                }
            }
            out[popped..].fill(0.0);
        }
        popped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spsc_transfers_samples_concurrently_without_corruption() {
        let (mut prod, mut cons) = SpscRingBuffer::with_capacity::<f32>(256);
        let total = 20_000usize;

        let producer_handle = std::thread::spawn(move || {
            let mut sent = 0usize;
            let mut chunk = [0.0f32; 64];
            while sent < total {
                let n = (total - sent).min(chunk.len());
                for (i, slot) in chunk.iter_mut().take(n).enumerate() {
                    *slot = (sent + i) as f32;
                }
                let written = prod.push_slice(&chunk[..n]);
                sent += written;
                if written == 0 {
                    std::thread::yield_now();
                }
            }
        });

        let mut received = 0usize;
        let mut buf = [0.0f32; 48];
        while received < total {
            let read = cons.pop_slice(&mut buf);
            for &sample in &buf[..read] {
                assert_eq!(sample, received as f32);
                received += 1;
            }
            if read == 0 {
                std::thread::yield_now();
            }
        }
        producer_handle.join().unwrap();
        assert_eq!(received, total);
        assert_eq!(cons.available(), 0);
    }
}
