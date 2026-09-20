//! Hard-bounded, insertion-ordered conformance trace capture.

use std::collections::{VecDeque, vec_deque};
use std::error::Error;
use std::fmt;
use std::num::NonZeroU16;

use crate::platform::contracts::{PlatformEvent, RequestCompletion};

/// Neutral hard bound for one conformance capture owner.
pub const MAX_CAPTURE_ITEMS: u16 = 4_096;

/// Invalid requested capture capacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CaptureLimitError {
    AboveHardLimit { requested: u16, maximum: u16 },
}

impl fmt::Display for CaptureLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("conformance capture capacity exceeds the hard bound")
    }
}

impl Error for CaptureLimitError {}

/// Rejection that returns ownership of an item when a capture is full.
pub struct CaptureCapacityError<T> {
    capacity: NonZeroU16,
    item: T,
}

impl<T> CaptureCapacityError<T> {
    pub const fn capacity(&self) -> NonZeroU16 {
        self.capacity
    }

    pub fn into_item(self) -> T {
        self.item
    }
}

impl<T> fmt::Debug for CaptureCapacityError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptureCapacityError")
            .field("capacity", &self.capacity)
            .field("item_redacted", &true)
            .finish()
    }
}

impl<T> fmt::Display for CaptureCapacityError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "conformance capture reached its {}-item capacity",
            self.capacity
        )
    }
}

impl<T> Error for CaptureCapacityError<T> {}

/// Bounded insertion-ordered owner used for deterministic event, request, and completion traces.
///
/// Saturation rejects and returns the new item. It never drops an older item, grows past its
/// declared capacity, coalesces entries, or dispatches work.
#[derive(Debug)]
pub struct BoundedCapture<T> {
    capacity: NonZeroU16,
    items: VecDeque<T>,
}

impl<T> BoundedCapture<T> {
    pub fn new(capacity: NonZeroU16) -> Result<Self, CaptureLimitError> {
        if capacity.get() > MAX_CAPTURE_ITEMS {
            return Err(CaptureLimitError::AboveHardLimit {
                requested: capacity.get(),
                maximum: MAX_CAPTURE_ITEMS,
            });
        }
        Ok(Self {
            capacity,
            items: VecDeque::with_capacity(capacity.get() as usize),
        })
    }

    pub const fn capacity(&self) -> NonZeroU16 {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn remaining_capacity(&self) -> usize {
        self.capacity.get() as usize - self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.items.len() == self.capacity.get() as usize
    }

    pub fn push(&mut self, item: T) -> Result<(), CaptureCapacityError<T>> {
        if self.is_full() {
            return Err(CaptureCapacityError {
                capacity: self.capacity,
                item,
            });
        }
        self.items.push_back(item);
        Ok(())
    }

    pub fn front(&self) -> Option<&T> {
        self.items.front()
    }

    pub fn back(&self) -> Option<&T> {
        self.items.back()
    }

    pub fn iter(&self) -> vec_deque::Iter<'_, T> {
        self.items.iter()
    }

    pub fn pop_front(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn take_all(&mut self) -> Vec<T> {
        self.items.drain(..).collect()
    }
}

/// Bounded trace of immutable platform events.
pub type EventCapture<T> = BoundedCapture<PlatformEvent<T>>;

/// Bounded trace of typed terminal request completions.
pub type CompletionCapture<T> = BoundedCapture<RequestCompletion<T>>;
