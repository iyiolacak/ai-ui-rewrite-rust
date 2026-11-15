//! Change notification (`Subscribable` upstream).
//!
//! Callbacks run after a state change completes, never while runtime state is
//! borrowed, so a callback may read the runtime or call back into it. A
//! callback added during notification first runs on the next change.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

type Callback = Rc<dyn Fn()>;

#[derive(Default)]
/// A list of change callbacks shared by runtimes and view layers.
pub struct Subscribers {
    next_id: Cell<u64>,
    callbacks: RefCell<Vec<(u64, Callback)>>,
}

impl Subscribers {
    /// Register a callback; dropping the returned guard unregisters it.
    pub fn subscribe(self: &Rc<Self>, callback: impl Fn() + 'static) -> Subscription {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.callbacks.borrow_mut().push((id, Rc::new(callback)));
        Subscription {
            list: Rc::downgrade(self),
            id,
        }
    }

    /// Call every registered callback (none is called while internal state is borrowed).
    pub fn notify(&self) {
        let callbacks: Vec<Callback> = self
            .callbacks
            .borrow()
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        for callback in callbacks {
            callback();
        }
    }
}

/// Keeps a callback registered; dropping it unsubscribes.
#[must_use = "dropping a Subscription unsubscribes immediately"]
pub struct Subscription {
    list: Weak<Subscribers>,
    id: u64,
}

impl Subscription {
    /// Keep the callback for the lifetime of the runtime.
    pub fn detach(self) {
        std::mem::forget(self);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(list) = self.list.upgrade() {
            list.callbacks.borrow_mut().retain(|(id, _)| *id != self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_guard_unsubscribes_and_reentrant_subscribe_is_safe() {
        let list = Rc::new(Subscribers::default());
        let count = Rc::new(Cell::new(0));
        let inner: Rc<RefCell<Vec<Subscription>>> = Rc::default();
        let guard = {
            let (count, list2, inner) = (count.clone(), list.clone(), inner.clone());
            list.subscribe(move || {
                count.set(count.get() + 1);
                inner.borrow_mut().push(list2.subscribe(|| {}));
            })
        };
        list.notify();
        assert_eq!(count.get(), 1);
        drop(guard);
        list.notify();
        assert_eq!(count.get(), 1);
    }
}
