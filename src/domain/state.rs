use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub const MIN_ATTENUATION_DB: f32 = 0.0;
pub const MAX_ATTENUATION_DB: f32 = 100.0;
pub const DEFAULT_ATTENUATION_DB: f32 = 100.0;

/// Lock-free atomic f32 storage using IEEE 754 bit-casting over AtomicU32.
#[derive(Debug)]
pub struct AtomicAttenuationLimit {
    bits: AtomicU32,
}

impl Default for AtomicAttenuationLimit {
    fn default() -> Self {
        Self::new(DEFAULT_ATTENUATION_DB)
    }
}

impl AtomicAttenuationLimit {
    pub fn new(initial_db: f32) -> Self {
        let clamped = Self::sanitize(initial_db);
        Self {
            bits: AtomicU32::new(clamped.to_bits()),
        }
    }

    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Acquire))
    }

    #[inline]
    pub fn set(&self, db: f32) {
        let clamped = Self::sanitize(db);
        self.bits.store(clamped.to_bits(), Ordering::Release);
    }

    #[inline]
    fn sanitize(val: f32) -> f32 {
        if val.is_nan() {
            DEFAULT_ATTENUATION_DB
        } else {
            val.clamp(MIN_ATTENUATION_DB, MAX_ATTENUATION_DB)
        }
    }
}

/// Shared engine state accessible by the audio core and UI threads.
#[derive(Debug)]
pub struct EngineState {
    pub attenuation_limit: AtomicAttenuationLimit,
    pub enabled: AtomicBool,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            attenuation_limit: AtomicAttenuationLimit::default(),
            enabled: AtomicBool::new(true),
        }
    }
}

impl EngineState {
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    #[inline]
    pub fn set_enabled(&self, state: bool) {
        self.enabled.store(state, Ordering::Release);
    }
}

pub static TERMINATE_SIGNAL: AtomicBool = AtomicBool::new(false);

extern "C" fn sig_handler(_sig: i32) {
    TERMINATE_SIGNAL.store(true, Ordering::Release);
}

/// Registers POSIX signal handlers for SIGINT and SIGTERM to enable graceful cleanup.
pub fn setup_signal_handlers() {
    unsafe {
        unsafe extern "C" {
            fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
        }
        const SIGINT: i32 = 2;
        const SIGTERM: i32 = 15;
        let _ = signal(SIGINT, sig_handler);
        let _ = signal(SIGTERM, sig_handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_attenuation_get_set() {
        let atten = AtomicAttenuationLimit::new(30.0);
        assert_eq!(atten.get(), 30.0);

        atten.set(65.5);
        assert_eq!(atten.get(), 65.5);
    }

    #[test]
    fn test_atomic_attenuation_clamping() {
        let atten = AtomicAttenuationLimit::new(150.0);
        assert_eq!(atten.get(), MAX_ATTENUATION_DB);

        atten.set(-10.0);
        assert_eq!(atten.get(), MIN_ATTENUATION_DB);

        atten.set(f32::NAN);
        assert_eq!(atten.get(), DEFAULT_ATTENUATION_DB);
    }

    #[test]
    fn test_engine_state_shared_access() {
        let state = EngineState::new_shared();
        assert!(state.is_enabled());
        assert_eq!(state.attenuation_limit.get(), DEFAULT_ATTENUATION_DB);

        state.set_enabled(false);
        assert!(!state.is_enabled());

        state.attenuation_limit.set(40.0);
        assert_eq!(state.attenuation_limit.get(), 40.0);
    }

    #[test]
    fn test_setup_signal_handlers_and_terminate_signal() {
        setup_signal_handlers();
        assert!(!TERMINATE_SIGNAL.load(Ordering::Acquire));
        TERMINATE_SIGNAL.store(true, Ordering::Release);
        assert!(TERMINATE_SIGNAL.load(Ordering::Acquire));
        TERMINATE_SIGNAL.store(false, Ordering::Release);
    }
}
