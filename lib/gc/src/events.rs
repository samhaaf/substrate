//! GcEvent — broadcast events emitted by GcService.

/// Reason an entry was evicted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvictionReason {
    TtlExpired,
    BudgetPressure,
    Forced,
}

/// Events that GcService broadcasts to subscribers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GcEvent {
    EntryRegistered {
        path: String,
        kind: String,
        size_bytes: u64,
        recovery_hint: Option<String>,
    },
    EntryTouched {
        path: String,
        touch_count: u64,
    },
    EntryLocked {
        path: String,
        lock_expires_at: i64,
    },
    EntryUnlocked {
        path: String,
    },
    EntryEvicting {
        path: String,
        reason: EvictionReason,
    },
    EntryEvicted {
        path: String,
        bytes_freed: u64,
        recovery_hint: Option<String>,
    },
    EntryEvictionFailed {
        path: String,
        error: String,
    },
    SweepCompleted {
        expired_evicted: u64,
        budget_evicted: u64,
        bytes_freed: u64,
    },
    DirRegistered {
        root: String,
        max_size_bytes: u64,
        default_ttl_secs: u64,
    },
    BudgetExceeded {
        dir: String,
        current_bytes: u64,
        max_bytes: u64,
    },
    MakeRoomCompleted {
        dir: String,
        bytes_freed: u64,
    },
    PathMoved {
        from: String,
        to: String,
    },
}
