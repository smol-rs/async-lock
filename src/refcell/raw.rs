use core::{
    cell::{Cell, RefCell},
    task::Waker,
};
use std::collections::VecDeque;

const BORROW_MUT_BIT: usize = 1;
const ONE_BORROW: usize = 2;

pub(super) struct RawRefCell {
    /// Event triggered when the last borrow is dropped.
    borrow_wakes: RefCell<VecDeque<Waker>>,

    /// Event triggered when the borrow_mut is dropped.
    borrow_mut_wakes: RefCell<VecDeque<Waker>>,

    /// Current state of the lock.
    ///
    /// The least significant bit (`WRITER_BIT`) is set to 1 when a writer is holding the lock or
    /// trying to acquire it.
    ///
    /// The upper bits contain the number of currently active borrows. Each active reader
    /// increments the state by `ONE_BORROW`.
    state: Cell<usize>,
}

impl RawRefCell {
    pub(super) const fn new() -> Self {
        Self {
            borrow_wakes: RefCell::new(VecDeque::new()),
            borrow_mut_wakes: RefCell::new(VecDeque::new()),
            state: Cell::new(0),
        }
    }

    pub(super) fn borrow(&self) -> RawBorrow<'_> {
        RawBorrow { lock: self }
    }

    pub(super) fn borrow_wake(&self, waker: Waker) {
        self.borrow_wakes.borrow_mut().push_back(waker);
    }

    pub(super) fn try_borrow(&self) -> bool {
        let state = self.state.get();

        // If there's a mutable borrow holding the lock or attempting to acquire it, we cannot acquire
        // a read lock here.
        if state & BORROW_MUT_BIT != 0 {
            return false;
        }

        // Make sure the number of borrows doesn't overflow.
        if state > isize::MAX as usize {
            crate::abort();
        }

        // Increment the number of readers.
        // TODO ? ok if self.state.update(|val| val + ONE_BORROW ) == state + ONE_BORROW;
        self.state.set(state + ONE_BORROW);

        true
    }

    pub(super) fn borrow_unlock(&self) {
        // Decrement the number of borrows.
        let state = self.state.get();
        self.state.set(state - ONE_BORROW);

        if self.state.get() == 0 {
            // If this was the last reader, wake up the next borrow the "no borrows" event.
            if let Some(borrow_mut_wake) = self.borrow_mut_wakes.borrow_mut().pop_front() {
                borrow_mut_wake.wake();
            }
        }
    }

    pub(super) fn borrow_mut(&self) -> RawBorrowMut<'_> {
        RawBorrowMut { lock: self }
    }

    pub(super) fn borrow_mut_wake(&self, waker: Waker) {
        self.borrow_mut_wakes.borrow_mut().push_back(waker);
    }

    pub(super) fn try_borrow_mut(&self) -> bool {
        let state = self.state.get();

        // If there's a mutable borrow holding the lock or attempting to acquire it, we cannot acquire
        // a borrow_mut lock here.
        if state & BORROW_MUT_BIT != 0 {
            return false;
        }

        // If there's at least one simple borrow, we cannot acquire
        // a borrow_mut lock here.
        if state & !BORROW_MUT_BIT != 0 {
            return false;
        }

        // Increment the number of readers.
        // TODO ? ok if self.state.update(|val| val & BORROW_MUT_BIT ) == BORROW_MUT_BIT;
        self.state.set(BORROW_MUT_BIT);

        true
    }

    pub(super) fn borrow_mut_unlock(&self) {
        if let Some(borrow_mut_wake) = self.borrow_mut_wakes.borrow_mut().pop_front() {
            // If there is a waiting borrow_mut, wake up
            borrow_mut_wake.wake();
        } else {
            // Only remove Borrow mut bit, if there is no other task waiting for it.
            let new_state = self.state.get() & !BORROW_MUT_BIT;
            self.state.set(new_state);
            // else, wakeup borrow_wake
            let mut borrow_wakes = self.borrow_wakes.borrow_mut();

            borrow_wakes
                .drain(0..)
                .for_each(|waiting_borrow| waiting_borrow.wake());
        }
    }
}

pub(super) struct RawBorrow<'a> {
    // The lock that is being acquired.
    pub(super) lock: &'a RawRefCell,
    // ??? // Making this type `!Unpin` enables future optimizations.
    // #[pin]
    // _pin: PhantomPinned
}

impl RawBorrow<'_> {
    pub fn try_borrow(&self) -> bool {
        self.lock.try_borrow()
    }
}

pub(super) struct RawBorrowMut<'a> {
    // The lock that is being acquired.
    pub(super) lock: &'a RawRefCell,
    // ??? // Making this type `!Unpin` enables future optimizations.
    // #[pin]
    // _pin: PhantomPinned
}

impl RawBorrowMut<'_> {
    pub fn try_borrow_mut(&self) -> bool {
        self.lock.try_borrow_mut()
    }
}
