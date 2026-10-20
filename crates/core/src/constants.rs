//! Limits the coordinator enforces. The app's own limits are in domain.rs.
pub const SLICE_COUNT: u8 = 64;
pub const MAX_PROJECT_RESOURCES: usize = SLICE_COUNT as usize + 1;
pub const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
pub const LEASE_TTL_SECS: i64 = 120;
