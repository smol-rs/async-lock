use event_listener::{Event, EventListener};

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::futures::Lock;
use crate::{Blocking, Mutex, NonBlocking, Strategy};

/// A counter to synchronize multiple tasks at the same time.
#[derive(Debug)]
pub struct Barrier {
    n: usize,
    state: Mutex<State>,
    event: Event,
}

#[derive(Debug)]
struct State {
    count: usize,
    generation_id: u64,
}

impl Barrier {
    /// Creates a barrier that can block the given number of tasks.
    ///
    /// A barrier will block `n`-1 tasks which call [`wait()`] and then wake up all tasks
    /// at once when the `n`th task calls [`wait()`].
    ///
    /// [`wait()`]: `Barrier::wait()`
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Barrier;
    ///
    /// let barrier = Barrier::new(5);
    /// ```
    pub const fn new(n: usize) -> Barrier {
        Barrier {
            n,
            state: Mutex::new(State {
                count: 0,
                generation_id: 0,
            }),
            event: Event::new(),
        }
    }

    /// Blocks the current task until all tasks reach this point.
    ///
    /// Barriers are reusable after all tasks have synchronized, and can be used continuously.
    ///
    /// Returns a [`BarrierWaitResult`] indicating whether this task is the "leader", meaning the
    /// last task to call this method.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Barrier;
    /// use futures_lite::future;
    /// use std::sync::Arc;
    /// use std::thread;
    ///
    /// let barrier = Arc::new(Barrier::new(5));
    ///
    /// for _ in 0..5 {
    ///     let b = barrier.clone();
    ///     thread::spawn(move || {
    ///         future::block_on(async {
    ///             // The same messages will be printed together.
    ///             // There will NOT be interleaving of "before" and "after".
    ///             println!("before wait");
    ///             b.wait().await;
    ///             println!("after wait");
    ///         });
    ///     });
    /// }
    /// ```
    pub fn wait(&self) -> BarrierWait<'_> {
        BarrierWait {
            barrier: self,
            lock: Some(self.state.lock()),
            state: WaitState::Initial,
        }
    }

    /// Blocks the current thread until all tasks reach this point.
    ///
    /// Barriers are reusable after all tasks have synchronized, and can be used continuously.
    ///
    /// Returns a [`BarrierWaitResult`] indicating whether this task is the "leader", meaning the
    /// last task to call this method.
    ///
    /// # Blocking
    ///
    /// Rather than using asynchronous waiting, like the [`wait`] method, this method will
    /// block the current thread until the wait is complete.
    ///
    /// This method should not be used in a synchronous context. It is intended to be
    /// used in a way that a barrier can be used in both asynchronous and synchronous contexts.
    /// Calling this method in an `async` function or block may result in a deadlock.
    ///
    /// # Examples
    ///
    /// ```
    /// use async_lock::Barrier;
    /// use futures_lite::future;
    /// use std::sync::Arc;
    /// use std::thread;
    ///
    /// let barrier = Arc::new(Barrier::new(5));
    ///
    /// for _ in 0..5 {
    ///     let b = barrier.clone();
    ///     thread::spawn(move || {
    ///         // The same messages will be printed together.
    ///         // There will NOT be interleaving of "before" and "after".
    ///         println!("before wait");
    ///         b.wait_blocking();
    ///         println!("after wait");
    ///     });
    /// }
    /// ```
    pub fn wait_blocking(&self) -> BarrierWaitResult {
        self.wait().wait()
    }
}

/// The future returned by [`Barrier::wait()`].
pub struct BarrierWait<'a> {
    /// The barrier to wait on.
    barrier: &'a Barrier,

    /// The ongoing mutex lock operation we are blocking on.
    lock: Option<Lock<'a, State>>,

    /// The current state of the future.
    state: WaitState,
}

impl fmt::Debug for BarrierWait<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BarrierWait { .. }")
    }
}

enum WaitState {
    /// We are getting the original values of the state.
    Initial,

    /// We are waiting for the listener to complete.
    Waiting {
        evl: Option<EventListener>,
        local_gen: u64,
    },

    /// Waiting to re-acquire the lock to check the state again.
    Reacquiring(u64),
}

impl BarrierWait<'_> {
    fn wait(mut self) -> BarrierWaitResult {
        match self.poll_with_strategy(&mut Blocking, &mut ()) {
            Poll::Ready(result) => result,
            Poll::Pending => unreachable!(),
        }
    }

    fn poll_with_strategy<S: Strategy>(
        &mut self,
        strategy: &mut S,
        cx: &mut S::Context,
    ) -> Poll<BarrierWaitResult> {
        loop {
            match self.state {
                WaitState::Initial => {
                    // See if the lock is ready yet.
                    let mut state =
                        ready!(self.lock.as_mut().unwrap().poll_with_strategy(strategy, cx));
                    self.lock = None;

                    let local_gen = state.generation_id;
                    state.count += 1;

                    if state.count < self.barrier.n {
                        // We need to wait for the event.
                        self.state = WaitState::Waiting {
                            evl: Some(self.barrier.event.listen()),
                            local_gen,
                        };
                    } else {
                        // We are the last one.
                        state.count = 0;
                        state.generation_id = state.generation_id.wrapping_add(1);
                        self.barrier.event.notify(std::usize::MAX);
                        return Poll::Ready(BarrierWaitResult { is_leader: true });
                    }
                }

                WaitState::Waiting {
                    ref mut evl,
                    local_gen,
                } => {
                    // Wait for the event to complete.
                    if let Err(e) = strategy.poll(cx, evl.take().unwrap()) {
                        *evl = Some(e);
                        return Poll::Pending;
                    }

                    // We are now re-acquiring the mutex.
                    self.lock = Some(self.barrier.state.lock());
                    self.state = WaitState::Reacquiring(local_gen);
                }

                WaitState::Reacquiring(local_gen) => {
                    // Acquire the local state again.
                    let state =
                        ready!(self.lock.as_mut().unwrap().poll_with_strategy(strategy, cx));
                    self.lock = None;

                    if local_gen == state.generation_id && state.count < self.barrier.n {
                        // We need to wait for the event again.
                        self.state = WaitState::Waiting {
                            evl: Some(self.barrier.event.listen()),
                            local_gen,
                        };
                    } else {
                        // We are ready, but not the leader.
                        return Poll::Ready(BarrierWaitResult { is_leader: false });
                    }
                }
            }
        }
    }
}

impl Future for BarrierWait<'_> {
    type Output = BarrierWaitResult;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut()
            .poll_with_strategy(&mut NonBlocking::default(), cx)
    }
}

/// Returned by [`Barrier::wait()`] when all tasks have called it.
///
/// # Examples
///
/// ```
/// # futures_lite::future::block_on(async {
/// use async_lock::Barrier;
///
/// let barrier = Barrier::new(1);
/// let barrier_wait_result = barrier.wait().await;
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct BarrierWaitResult {
    is_leader: bool,
}

impl BarrierWaitResult {
    /// Returns `true` if this task was the last to call to [`Barrier::wait()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # futures_lite::future::block_on(async {
    /// use async_lock::Barrier;
    /// use futures_lite::future;
    ///
    /// let barrier = Barrier::new(2);
    /// let (a, b) = future::zip(barrier.wait(), barrier.wait()).await;
    /// assert_eq!(a.is_leader(), false);
    /// assert_eq!(b.is_leader(), true);
    /// # });
    /// ```
    pub fn is_leader(&self) -> bool {
        self.is_leader
    }
}
