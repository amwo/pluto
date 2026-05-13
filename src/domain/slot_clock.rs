use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const SLOT_MS: i64 = 400;

static CLOCK: OnceLock<SlotClock> = OnceLock::new();

#[derive(Clone, Copy, Debug)]
pub struct SlotClock {
    ref_slot: u64,
    ref_unix_ms: i64,
}

impl SlotClock {
    pub fn new(ref_slot: u64, ref_time: SystemTime) -> Self {
        let ref_unix_ms = ref_time
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self { ref_slot, ref_unix_ms }
    }

    pub fn estimate_block_time_ms(&self, slot: u64) -> i64 {
        let diff_slots = slot as i64 - self.ref_slot as i64;
        self.ref_unix_ms + diff_slots * SLOT_MS
    }

    pub fn detection_delay_ms(&self, slot: u64, now: SystemTime) -> i64 {
        let now_ms = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        now_ms - self.estimate_block_time_ms(slot)
    }
}

pub fn init(ref_slot: u64) {
    let _ = CLOCK.set(SlotClock::new(ref_slot, SystemTime::now()));
}

pub fn detection_delay_ms(slot: u64) -> Option<i32> {
    CLOCK
        .get()
        .map(|c| c.detection_delay_ms(slot, SystemTime::now()) as i32)
}
