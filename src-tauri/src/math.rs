use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq)]
pub enum InvariantViolation {
    MemoryLimitExceeded { actual: usize, limit: usize },
    TimeWindowExceeded { actual: u64, limit: u64 },
    NonMonotonicTimestamps { previous: u64, current: u64 },
    ByteSumMismatch { recorded: usize, calculated: usize },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CapacityModel {
    pub bitrate_mbps: f64,
    pub ram_limit_mb: u32,
    pub theoretical_max_minutes: f64,
}

impl CapacityModel {
    pub fn calculate(ram_limit_mb: u32, bitrate_mbps: f64) -> Self {
        let safe_bitrate = bitrate_mbps.max(0.1);
        let bits_available = (ram_limit_mb as f64) * 1024.0 * 1024.0 * 8.0;
        let bits_per_second = safe_bitrate * 1_000_000.0;
        let max_seconds = bits_available / bits_per_second;
        let theoretical_max_minutes = max_seconds / 60.0;

        Self {
            bitrate_mbps: safe_bitrate,
            ram_limit_mb,
            theoretical_max_minutes,
        }
    }
}

pub fn verify_buffer_invariants(
    init_header_len: usize,
    chunks: &[(usize, f64)],
    recorded_total: usize,
    limit_bytes: usize,
    max_duration_secs: f64,
) -> Result<(), InvariantViolation> {
    let mut sum = init_header_len;
    let mut last_ts = 0.0;

    for (i, &(len, ts)) in chunks.iter().enumerate() {
        if i > 0 && ts < last_ts {
            return Err(InvariantViolation::NonMonotonicTimestamps {
                previous: (last_ts * 1000.0) as u64,
                current: (ts * 1000.0) as u64,
            });
        }
        last_ts = ts;
        sum += len;
    }

    if sum != recorded_total {
        return Err(InvariantViolation::ByteSumMismatch {
            recorded: recorded_total,
            calculated: sum,
        });
    }

    let header_tolerance = 256 * 1024;
    if recorded_total > (limit_bytes + header_tolerance) {
        return Err(InvariantViolation::MemoryLimitExceeded {
            actual: recorded_total,
            limit: limit_bytes + header_tolerance,
        });
    }

    if let (Some(first), Some(last)) = (chunks.first(), chunks.last()) {
        let span = last.1 - first.1;
        let time_tolerance = 10.0;
        if max_duration_secs > 0.0 && span > (max_duration_secs + time_tolerance) {
            return Err(InvariantViolation::TimeWindowExceeded {
                actual: span as u64,
                limit: (max_duration_secs + time_tolerance) as u64,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capacity_model_math() {
        let model = CapacityModel::calculate(500, 6.0);
        let expected_seconds = (500.0 * 1024.0 * 1024.0 * 8.0) / 6_000_000.0;
        let expected_min = expected_seconds / 60.0;
        assert!((model.theoretical_max_minutes - expected_min).abs() < 0.01);
    }

    #[test]
    fn test_invariants_pass_under_normal_conditions() {
        let chunks = vec![(1000, 10.0), (2000, 15.0), (3000, 20.0)];
        let res = verify_buffer_invariants(500, &chunks, 6500, 10000, 60.0);
        assert!(res.is_ok());
    }

    #[test]
    fn test_invariants_fail_on_non_monotonic_time() {
        let chunks = vec![(1000, 20.0), (2000, 15.0)];
        let res = verify_buffer_invariants(0, &chunks, 3000, 10000, 60.0);
        assert!(matches!(res, Err(InvariantViolation::NonMonotonicTimestamps { .. })));
    }
}
