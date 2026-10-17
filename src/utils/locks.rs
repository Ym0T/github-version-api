//! One async mutex per key. A lock stays in the map as long as somebody
//! holds a reference to it, so it can never be evicted while in use.
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, PoisonError, Weak};

use tokio::sync::Mutex;

#[derive(Default)]
pub struct KeyedLocks {
    locks: StdMutex<HashMap<String, Weak<Mutex<()>>>>,
}

impl KeyedLocks {
    pub fn get(&self, key: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
            return lock;
        }
        locks.retain(|_, lock| lock.strong_count() > 0);
        let lock = Arc::new(Mutex::new(()));
        locks.insert(key.to_string(), Arc::downgrade(&lock));
        lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_same_lock_until_released() {
        let locks = KeyedLocks::default();
        let a = locks.get("a");
        assert!(Arc::ptr_eq(&a, &locks.get("a")));
        assert!(!Arc::ptr_eq(&a, &locks.get("b")));
        drop(a);
        // Released locks are pruned.
        let _c = locks.get("c");
        assert_eq!(locks.locks.lock().unwrap().len(), 1);
    }
}
