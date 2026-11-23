//! Tiny wall-clock profiler for hunting slow spots (enabled by calling `prof::report` from a bin).
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const NAMES: [&str; 8] = ["plan_payment", "recompute_layers", "legal_actions", "run_sba", "checkpoint", "other1", "other2", "other3"];
pub static NS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
pub static ENTERED: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];
pub static CALLS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

pub struct Guard(usize, Instant);
pub fn enter(i: usize) -> Guard {
    ENTERED[i].fetch_add(1, Ordering::Relaxed);
    Guard(i, Instant::now())
}
impl Drop for Guard {
    fn drop(&mut self) {
        NS[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
        CALLS[self.0].fetch_add(1, Ordering::Relaxed);
    }
}

pub fn report() -> String {
    NAMES.iter().enumerate().map(|(i, n)| format!("{n}: {} entered, {} calls, {:.2}s", ENTERED[i].load(Ordering::Relaxed), CALLS[i].load(Ordering::Relaxed), NS[i].load(Ordering::Relaxed) as f64 / 1e9)).collect::<Vec<_>>().join("\n") + &format!("\nLAST plan: {}", LAST.lock().map(|l| l.clone()).unwrap_or_default())
}

pub static LAST: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
