/// Lamport 逻辑时钟（分布式排序用）。
///
/// 设计文档 §5：排序靠 `(lamport, device_id)`。本机每次捕获 `tick()` 取递增计数器；
/// 收到对端信封时 `observe(remote)` 取 `max(local, remote) + 1`，保证因果偏序。
#[derive(Debug, Clone, Copy, Default)]
pub struct LamportClock {
    counter: u64,
}

impl LamportClock {
    pub fn new() -> Self {
        Self { counter: 0 }
    }

    /// 本机事件：计数器 +1 并返回新值。
    pub fn tick(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    /// 收到对端逻辑时钟：取较大者 +1，维持跨机递增。
    pub fn observe(&mut self, remote: u64) {
        self.counter = self.counter.max(remote) + 1;
    }

    /// 当前计数器值（不推进）。
    pub fn current(&self) -> u64 {
        self.counter
    }
}

/// `(lamport, device_id)` 排序键：用于把条目排成稳定偏序。
/// 返回元组以便直接 `sort_by_key`。
pub fn sort_key(lamport: u64, device_id: &str) -> (u64, String) {
    (lamport, device_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_increments() {
        let mut c = LamportClock::new();
        assert_eq!(c.tick(), 1);
        assert_eq!(c.tick(), 2);
        assert_eq!(c.current(), 2);
    }

    #[test]
    fn observe_takes_max_plus_one() {
        let mut c = LamportClock::new();
        c.tick(); // -> 1
        c.observe(5); // -> max(1,5)+1 = 6
        assert_eq!(c.current(), 6);
        c.observe(2); // -> max(6,2)+1 = 7
        assert_eq!(c.current(), 7);
    }

    #[test]
    fn order_by_lamport_then_device() {
        let mut v = vec![
            sort_key(2, "devB"),
            sort_key(1, "devA"),
            sort_key(2, "devA"),
        ];
        v.sort();
        assert_eq!(v, vec![sort_key(1, "devA"), sort_key(2, "devA"), sort_key(2, "devB")]);
    }
}
