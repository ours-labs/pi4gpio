//! Per-bus lock ownership for multiplexing multiple daemon clients.
//!
//! Locks are released explicitly or when the owning client disconnects.

use crate::client::ClientId;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusId {
    Gpio(u32),
    I2c(u8),
    Spi(u8, u8),
    Uart(u8),
}

#[derive(Default)]
pub struct LockTable {
    holders: Mutex<HashMap<BusId, ClientId>>,
}

impl LockTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(&self, bus: BusId, client: ClientId) -> Result<(), ClientId> {
        let mut holders = self.holders.lock().expect("lock table poisoned");
        match holders.get(&bus) {
            Some(existing) if *existing != client => Err(existing.clone()),
            _ => {
                holders.insert(bus, client);
                Ok(())
            }
        }
    }

    pub fn release_with<F>(&self, bus: BusId, client: &ClientId, before_unlock: F) -> bool
    where
        F: FnOnce(),
    {
        let mut holders = self.holders.lock().expect("lock table poisoned");
        if holders.get(&bus) == Some(client) {
            before_unlock();
            holders.remove(&bus);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(pid: u32) -> ClientId {
        ClientId::Local {
            uid: 1000,
            pid,
            session_id: pid as u64,
        }
    }

    fn session(pid: u32, session_id: u64) -> ClientId {
        ClientId::Local {
            uid: 1000,
            pid,
            session_id,
        }
    }

    #[test]
    fn same_client_can_reacquire_a_bus() {
        let locks = LockTable::new();
        let owner = client(10);
        let bus = BusId::Uart(0);

        assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        assert_eq!(locks.try_acquire(bus, owner), Ok(()));
    }

    #[test]
    fn another_client_cannot_take_a_held_bus() {
        let locks = LockTable::new();
        let owner = client(10);
        let contender = client(20);
        let bus = BusId::I2c(1);

        assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        assert_eq!(locks.try_acquire(bus, contender), Err(owner));
    }

    #[test]
    fn non_owner_release_does_not_unlock_the_bus() {
        let locks = LockTable::new();
        let owner = client(10);
        let contender = client(20);
        let bus = BusId::Gpio(17);

        assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        assert!(!locks.release_with(bus, &contender, || {}));
        assert_eq!(locks.try_acquire(bus, contender), Err(owner));
    }

    #[test]
    fn reconnect_from_same_process_is_a_distinct_lock_owner() {
        let locks = LockTable::new();
        let old_session = session(10, 1);
        let new_session = session(10, 2);
        let bus = BusId::Uart(0);

        assert_eq!(locks.try_acquire(bus, old_session.clone()), Ok(()));
        assert_eq!(
            locks.try_acquire(bus, new_session),
            Err(old_session.clone())
        );
        assert!(locks.release_with(bus, &old_session, || {}));
    }

    #[test]
    fn non_owner_release_does_not_run_cleanup() {
        let locks = LockTable::new();
        let owner = client(10);
        let contender = client(20);
        let bus = BusId::I2c(1);
        let mut cleanup_ran = false;

        assert_eq!(locks.try_acquire(bus, owner.clone()), Ok(()));
        assert!(!locks.release_with(bus, &contender, || cleanup_ran = true));
        assert!(!cleanup_ran);
        assert_eq!(locks.try_acquire(bus, contender), Err(owner));
    }

    #[test]
    fn disconnect_cleanup_releases_every_bus_owned_by_the_client() {
        let locks = LockTable::new();
        let disconnected = client(10);
        let next_client = client(20);
        let held = [BusId::Gpio(6), BusId::Spi(0, 0), BusId::Uart(0)];

        for bus in held {
            assert_eq!(locks.try_acquire(bus, disconnected.clone()), Ok(()));
        }
        for bus in held {
            assert!(locks.release_with(bus, &disconnected, || {}));
        }
        for bus in held {
            assert_eq!(locks.try_acquire(bus, next_client.clone()), Ok(()));
        }
    }
}
