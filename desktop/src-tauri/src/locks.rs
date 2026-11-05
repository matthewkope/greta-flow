//! Poison recovery at shared-state boundaries. Repair uncertain state before
//! clearing poison; never turn a previous worker panic into a callback panic.
use std::sync::{Mutex, MutexGuard};

pub trait MutexExt<T> {
    fn lock_recover(&self) -> MutexGuard<'_, T> {
        self.lock_repair(|_| {})
    }
    fn lock_repair(&self, repair: impl FnOnce(&mut T)) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_repair(&self, repair: impl FnOnce(&mut T)) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::warn!("recovering poisoned shared state");
                let mut guard = poisoned.into_inner();
                repair(&mut guard);
                self.clear_poison();
                guard
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_once_before_the_next_use() {
        let lock = Mutex::new(vec![1]);
        let _ = std::panic::catch_unwind(|| {
            lock.lock().unwrap().push(2);
            let _guard = lock.lock().unwrap();
            panic!("interrupted mutation");
        });
        assert!(lock.is_poisoned());
        lock.lock_repair(Vec::clear).push(3);
        assert!(!lock.is_poisoned());
        assert_eq!(
            *lock.lock_repair(|_| panic!("must not repair twice")),
            vec![3]
        );
    }
}
