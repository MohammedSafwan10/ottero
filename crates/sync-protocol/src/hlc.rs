//! Hybrid Logical Clock (HLC) for total deterministic ordering across devices.
//!
//! Combines physical wall time (milliseconds since Unix epoch), a 16-bit logical
//! counter, and a stable device identifier. Formatted as
//! `<13-digit-zero-padded-ms>-<4-hex-counter>-<device_id>` so lexicographical
//! string ordering in SQLite matches Rust's `Ord` implementation.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::collection::validate_ident;
use crate::{ProtocolError, ProtocolResult};

/// Maximum allowed forward clock skew when merging a remote HLC (60 seconds).
pub const MAX_CLOCK_DRIFT_MS: i64 = 60_000;

/// Maximum supported millisecond timestamp (`9999-12-31` fits comfortably in 13 digits).
const MAX_WALL_MS: i64 = 9_999_999_999_999;

/// One point in Hybrid Logical Time.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    pub wall_ms: i64,
    pub counter: u16,
    pub device_id: String,
}

impl Hlc {
    pub fn new(wall_ms: i64, counter: u16, device_id: impl Into<String>) -> ProtocolResult<Self> {
        let device_id = device_id.into();
        if !(0..=MAX_WALL_MS).contains(&wall_ms) {
            return Err(ProtocolError::InvalidHlc);
        }
        validate_ident(&device_id, "device_id", 64)?;
        Ok(Self {
            wall_ms,
            counter,
            device_id,
        })
    }

    pub fn validate(&self) -> ProtocolResult<()> {
        if !(0..=MAX_WALL_MS).contains(&self.wall_ms) {
            return Err(ProtocolError::InvalidHlc);
        }
        validate_ident(&self.device_id, "device_id", 64)?;
        Ok(())
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:013}-{:04x}-{}",
            self.wall_ms, self.counter, self.device_id
        )
    }
}

impl FromStr for Hlc {
    type Err = ProtocolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(3, '-');
        let wall_str = parts.next().ok_or(ProtocolError::InvalidHlc)?;
        let counter_str = parts.next().ok_or(ProtocolError::InvalidHlc)?;
        let device_id = parts.next().ok_or(ProtocolError::InvalidHlc)?;
        if wall_str.len() != 13 || counter_str.len() != 4 {
            return Err(ProtocolError::InvalidHlc);
        }
        let wall_ms = wall_str
            .parse::<i64>()
            .map_err(|_| ProtocolError::InvalidHlc)?;
        let counter =
            u16::from_str_radix(counter_str, 16).map_err(|_| ProtocolError::InvalidHlc)?;
        Self::new(wall_ms, counter, device_id)
    }
}

impl Serialize for Hlc {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Hlc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Stateful per-device HLC generator. Callers pass `now_ms` explicitly (no IO).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlcClock {
    device_id: String,
    last_wall_ms: i64,
    last_counter: u16,
}

impl HlcClock {
    pub fn new(device_id: impl Into<String>) -> ProtocolResult<Self> {
        let device_id = device_id.into();
        validate_ident(&device_id, "device_id", 64)?;
        Ok(Self {
            device_id,
            last_wall_ms: 0,
            last_counter: 0,
        })
    }

    pub fn from_last(device_id: impl Into<String>, last: &Hlc) -> ProtocolResult<Self> {
        let device_id = device_id.into();
        validate_ident(&device_id, "device_id", 64)?;
        last.validate()?;
        Ok(Self {
            device_id,
            last_wall_ms: last.wall_ms,
            last_counter: last.counter,
        })
    }

    /// Generates a strictly increasing HLC for a local write at `now_ms`.
    pub fn tick(&mut self, now_ms: i64) -> ProtocolResult<Hlc> {
        if !(0..=MAX_WALL_MS).contains(&now_ms) {
            return Err(ProtocolError::InvalidHlc);
        }
        if now_ms > self.last_wall_ms {
            self.last_wall_ms = now_ms;
            self.last_counter = 0;
        } else {
            self.last_counter = self
                .last_counter
                .checked_add(1)
                .ok_or(ProtocolError::CounterOverflow)?;
        }
        Hlc::new(self.last_wall_ms, self.last_counter, self.device_id.clone())
    }

    /// Merges a received remote HLC at `now_ms`, rejecting excessive forward drift.
    pub fn receive(&mut self, remote: &Hlc, now_ms: i64) -> ProtocolResult<Hlc> {
        remote.validate()?;
        if !(0..=MAX_WALL_MS).contains(&now_ms) {
            return Err(ProtocolError::InvalidHlc);
        }
        if remote.wall_ms > now_ms.saturating_add(MAX_CLOCK_DRIFT_MS) {
            return Err(ProtocolError::ClockDrift);
        }

        let prev_wall = self.last_wall_ms;
        let next_wall = now_ms.max(prev_wall).max(remote.wall_ms);

        let next_counter = if next_wall == prev_wall && next_wall == remote.wall_ms {
            self.last_counter
                .max(remote.counter)
                .checked_add(1)
                .ok_or(ProtocolError::CounterOverflow)?
        } else if next_wall == prev_wall {
            self.last_counter
                .checked_add(1)
                .ok_or(ProtocolError::CounterOverflow)?
        } else if next_wall == remote.wall_ms {
            remote
                .counter
                .checked_add(1)
                .ok_or(ProtocolError::CounterOverflow)?
        } else {
            0
        };

        self.last_wall_ms = next_wall;
        self.last_counter = next_counter;
        Hlc::new(self.last_wall_ms, self.last_counter, self.device_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_stays_monotonic_even_when_wall_clock_moves_backwards() {
        let mut clock = HlcClock::new("dev-a").expect("clock");
        let t1 = clock.tick(1_760_000_000_000).expect("t1");
        let t2 = clock.tick(1_760_000_000_000).expect("t2");
        let t3 = clock.tick(1_759_999_999_000).expect("t3");
        let t4 = clock.tick(1_760_000_000_005).expect("t4");

        assert!(t1 < t2);
        assert!(t2 < t3);
        assert!(t3 < t4);
        assert_eq!(t1.counter, 0);
        assert_eq!(t2.counter, 1);
        assert_eq!(t3.counter, 2);
        assert_eq!(t4.counter, 0);
    }

    #[test]
    fn receive_merges_remote_clocks_and_rejects_excessive_drift() {
        let mut clock = HlcClock::new("dev-b").expect("clock");
        let remote = Hlc::new(1_760_000_010_000, 4, "dev-a").expect("remote");
        let merged = clock.receive(&remote, 1_760_000_000_000).expect("merge");
        assert!(merged > remote);
        assert_eq!(merged.wall_ms, 1_760_000_010_000);
        assert_eq!(merged.counter, 5);

        // Remote clock > 60s ahead of local time is rejected.
        let far_future = Hlc::new(1_760_000_100_000, 0, "dev-a").expect("future");
        assert_eq!(
            clock.receive(&far_future, 1_760_000_000_000),
            Err(ProtocolError::ClockDrift)
        );
    }

    #[test]
    fn string_representation_matches_ord_and_round_trips() {
        let a = Hlc::new(1_760_000_000_000, 9, "dev-a").expect("a");
        let b = Hlc::new(1_760_000_000_000, 10, "dev-a").expect("b");
        let c = Hlc::new(1_760_000_000_000, 10, "dev-b").expect("c");
        let d = Hlc::new(1_760_000_000_001, 0, "dev-a").expect("d");

        for (lo, hi) in [(&a, &b), (&b, &c), (&c, &d)] {
            assert!(lo < hi);
            assert!(lo.to_string() < hi.to_string());
        }
        let parsed: Hlc = b.to_string().parse().expect("parse");
        assert_eq!(parsed, b);
    }
}
