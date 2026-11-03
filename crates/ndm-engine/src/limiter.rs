//! Limiteur de débit à seau de jetons, partageable entre connexions.
//!
//! Chaque connexion appelle `acquire(n)` après avoir reçu `n` octets.
//! Le seau peut passer en négatif (« dette ») : l'appelant dort alors le temps
//! nécessaire pour la rembourser, ce qui lisse le débit sans bloquer les autres.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RateLimiter {
    /// Octets par seconde, 0 = illimité.
    rate: AtomicU64,
    state: Mutex<BucketState>,
}

#[derive(Debug)]
struct BucketState {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            rate: AtomicU64::new(bytes_per_sec),
            state: Mutex::new(BucketState { tokens: 0.0, last: Instant::now() }),
        }
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
        let mut s = self.state.lock();
        // On repart d'un seau neutre pour que le nouveau réglage s'applique tout de suite.
        s.tokens = 0.0;
        s.last = Instant::now();
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Calcule l'attente nécessaire sans dormir (utile pour combiner plusieurs limiteurs).
    pub fn reserve(&self, n: u64) -> Duration {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return Duration::ZERO;
        }
        let rate_f = rate as f64;
        let mut s = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(s.last).as_secs_f64();
        s.last = now;
        // Rafale maximale : 250 ms de débit.
        let burst = rate_f * 0.25;
        s.tokens = (s.tokens + elapsed * rate_f).min(burst);
        s.tokens -= n as f64;
        if s.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((-s.tokens / rate_f).min(5.0))
        }
    }

    pub async fn acquire(&self, n: u64) {
        let wait = self.reserve(n);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Attend le plus long des délais imposés par plusieurs limiteurs (global + fichier).
pub async fn acquire_all(limiters: &[&RateLimiter], n: u64) {
    let wait = limiters.iter().map(|l| l.reserve(n)).max().unwrap_or_default();
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unlimited_never_waits() {
        let l = RateLimiter::new(0);
        assert_eq!(l.reserve(10_000_000), Duration::ZERO);
    }

    #[tokio::test]
    async fn limits_throughput() {
        let l = RateLimiter::new(100_000); // 100 Ko/s
        let t = Instant::now();
        for _ in 0..30 {
            l.acquire(10_000).await; // 300 Ko au total
        }
        let secs = t.elapsed().as_secs_f64();
        // ~3 s attendues (moins la rafale initiale nulle) ; tolérance large pour la CI.
        assert!(secs > 2.4 && secs < 4.0, "durée {secs}");
    }
}
