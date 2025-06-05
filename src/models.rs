use std::sync::atomic::AtomicUsize;

// Fake names for verification
pub const FAKE_FIRST_NAME: &str = "fmaksfnsa";
pub const FAKE_LAST_NAME: &str = "fjiqwfn91wf";
pub const MAX_RETRIES: usize = 1000;

pub struct Counters {
    pub requests: AtomicUsize,
    pub success: AtomicUsize,
    pub errors: AtomicUsize,
    pub ratelimits: AtomicUsize,
    pub hits: AtomicUsize
}

impl Counters {
    // Add a new() method for easier creation
    pub fn new() -> Self {
        Self {
            requests: AtomicUsize::new(0),
            success: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
            ratelimits: AtomicUsize::new(0),
            hits: AtomicUsize::new(0)
        }
    }
}