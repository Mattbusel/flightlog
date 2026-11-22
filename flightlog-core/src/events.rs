//! Flight events: mode changes, arming, failsafes, failure detector trips
//! and firmware warnings, in one list for ArduPilot and PX4 logs.
//!
//! For PX4 ([`from_ulog`]) the events come from `vehicle_status`
//! (`nav_state`, `arming_state`, `failsafe`, `failure_detector_status`),
//! `actuator_armed` when it is logged, and console messages of level
//! WARNING or worse. For ArduPilot ([`from_log`], feature `facts`) they are
//! the flight-facts events: `ERR`, arming, `MODE` changes and failsafe texts.
//!
//! ```no_run
//! use flightlog_core::{events, ulog::ULog};
//!
//! let log = ULog::open("crash.ulg")?;
//! for e in events::from_ulog(&log) {
//!     println!("{:>10.3} s {:?} {}", e.time_us as f64 / 1e6, e.kind, e.text);
//! }
//! # Ok::<(), flightlog_core::Error>(())
//! ```

use crate::ulog::ULog;

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventKind {
    /// Flight mode changed (also the mode at the start of the log).
    Mode,
    /// The vehicle armed.
    Armed,
    /// The vehicle disarmed.
    Disarmed,
    /// A failsafe started (PX4 `vehicle_status.failsafe`).
    Failsafe,
    /// The failsafe ended (PX4).
    FailsafeCleared,
    /// PX4's failure detector tripped (`failure_detector_status` became
    /// non-zero): attitude beyond `FD_FAIL_R` / `FD_FAIL_P`, external
    /// trigger, ESC arming failure, ...
    FailureDetector,
    /// ArduPilot `ERR` message (subsystem and code in `code`).
    Error,
    /// PX4: a console message of level WARNING or worse. ArduPilot: a
    /// failsafe, GPS glitch, EKF or crash text (`MSG` / `STATUSTEXT`).
    Message,
}

impl EventKind {
    /// Lower-case name: `"mode"`, `"armed"`, `"disarmed"`, `"failsafe"`,
    /// `"failsafe_cleared"`, `"failure_detector"`, `"error"`, `"message"`.
    pub fn name(self) -> &'static str {
        match self {
            EventKind::Mode => "mode",
            EventKind::Armed => "armed",
            EventKind::Disarmed => "disarmed",
            EventKind::Failsafe => "failsafe",
            EventKind::FailsafeCleared => "failsafe_cleared",
            EventKind::FailureDetector => "failure_detector",
            EventKind::Error => "error",
            EventKind::Message => "message",
        }
    }
}

/// One event.
#[derive(Debug, Clone, PartialEq)]
pub struct FlightEvent {
    /// Timestamp in the log's time base.
    pub time_us: u64,
    /// What happened.
    pub kind: EventKind,
    /// Human-readable description (mode name, message text, ...).
    pub text: String,
    /// Mode number, PX4 failure bits, console level, or ArduPilot ERR subsystem.
    pub code: Option<i64>,
}

/// PX4 `vehicle_status.nav_state` names. Values that PX4 reassigned over
/// releases (6 to 9, 11, 16) are left as numbers.
pub fn px4_nav_state_name(n: i64) -> Option<&'static str> {
    Some(match n {
        0 => "Manual",
        1 => "Altitude",
        2 => "Position",
        3 => "Mission",
        4 => "Hold",
        5 => "Return",
        10 => "Acro",
        12 => "Descend",
        13 => "Termination",
        14 => "Offboard",
        15 => "Stabilized",
        17 => "Takeoff",
        18 => "Land",
        19 => "Follow Me",
        20 => "Precision Land",
        21 => "Orbit",
        22 => "VTOL Takeoff",
        _ => return None,
    })
}

/// Names of the set bits of PX4 `failure_detector_status`.
pub fn px4_failure_bits(bits: i64) -> Vec<&'static str> {
    const NAMES: [(i64, &str); 8] = [
        (1, "roll"),
        (2, "pitch"),
        (4, "altitude"),
        (8, "external"),
        (16, "arm_esc"),
        (32, "battery"),
        (64, "imbalanced_prop"),
        (128, "motor"),
    ];
    NAMES
        .iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| *n)
        .collect()
}

/// Events of a PX4 ULog, in time order.
pub fn from_ulog(log: &ULog) -> Vec<FlightEvent> {
    let mut ev = Vec::new();
    let push = |ev: &mut Vec<FlightEvent>, t: i64, kind, text: String, code| {
        ev.push(FlightEvent {
            time_us: t.max(0) as u64,
            kind,
            text,
            code,
        })
    };
    let armed_topic = log.table("actuator_armed").map(|t| t.instance(0));
    if let Some(vs) = log.table("vehicle_status").map(|t| t.instance(0)) {
        let col = |n: &str| vs.column(n);
        let ts = col("timestamp");
        let (mut mode, mut armed, mut fs, mut fd) = (None, false, false, 0i64);
        for i in 0..vs.len() {
            let Some(t) = ts.and_then(|c| c.data.get_i64(i)) else {
                continue;
            };
            if let Some(n) = col("nav_state").and_then(|c| c.data.get_i64(i)) {
                if mode != Some(n) {
                    mode = Some(n);
                    let name = px4_nav_state_name(n)
                        .map(String::from)
                        .unwrap_or_else(|| format!("nav_state {n}"));
                    push(&mut ev, t, EventKind::Mode, name, Some(n));
                }
            }
            if armed_topic.is_none() {
                // ARMING_STATE_ARMED is 2 in every PX4 release
                if let Some(a) = col("arming_state").and_then(|c| c.data.get_i64(i)) {
                    let now = a == 2;
                    if now != armed {
                        armed = now;
                        let k = if now {
                            EventKind::Armed
                        } else {
                            EventKind::Disarmed
                        };
                        push(&mut ev, t, k, k.name().into(), None);
                    }
                }
            }
            if let Some(f) = col("failsafe").and_then(|c| c.data.get_i64(i)) {
                let now = f != 0;
                if now != fs {
                    fs = now;
                    let k = if now {
                        EventKind::Failsafe
                    } else {
                        EventKind::FailsafeCleared
                    };
                    push(&mut ev, t, k, k.name().replace('_', " "), None);
                }
            }
            if let Some(b) = col("failure_detector_status").and_then(|c| c.data.get_i64(i)) {
                if b != fd {
                    // report newly set bits only
                    let new = b & !fd;
                    fd = b;
                    if new != 0 {
                        let names = px4_failure_bits(new);
                        let text = if names.is_empty() {
                            format!("failure detector: bits {new:#x}")
                        } else {
                            format!("failure detector: {}", names.join(", "))
                        };
                        push(&mut ev, t, EventKind::FailureDetector, text, Some(new));
                    }
                }
            }
        }
    }
    if let Some(aa) = armed_topic {
        let mut armed = false;
        for i in 0..aa.len() {
            let t = aa.column("timestamp").and_then(|c| c.data.get_i64(i));
            let a = aa.column("armed").and_then(|c| c.data.get_i64(i));
            if let (Some(t), Some(a)) = (t, a) {
                let now = a != 0;
                if now != armed {
                    armed = now;
                    let k = if now {
                        EventKind::Armed
                    } else {
                        EventKind::Disarmed
                    };
                    push(&mut ev, t, k, k.name().into(), None);
                }
            }
        }
    }
    for m in &log.logged {
        if m.level <= 4 {
            push(
                &mut ev,
                m.timestamp_us as i64,
                EventKind::Message,
                format!("[{}] {}", m.level_name(), m.text),
                Some(m.level as i64),
            );
        }
    }
    ev.sort_by_key(|e| e.time_us);
    ev
}

/// Events of an ArduPilot DataFlash log or tlog: the flight-facts events
/// (`ERR`, arming, `MODE` changes, failsafe texts), in time order.
#[cfg(feature = "facts")]
pub fn from_log(log: &crate::Log) -> Vec<FlightEvent> {
    let opts = crate::facts::Options {
        events: true,
        ..Default::default()
    };
    let mut ev: Vec<FlightEvent> = crate::facts::run(log, &opts)
        .events
        .into_iter()
        .map(|e| {
            let kind = match e.kind.as_str() {
                "mode" => EventKind::Mode,
                "armed" => EventKind::Armed,
                "disarmed" => EventKind::Disarmed,
                "error" => EventKind::Error,
                // failsafe, glitch and crash texts
                _ => EventKind::Message,
            };
            FlightEvent {
                time_us: e.time_us,
                kind,
                text: e.text,
                code: e.code,
            }
        })
        .collect();
    ev.sort_by_key(|e| e.time_us);
    ev
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_bits() {
        assert_eq!(px4_failure_bits(3), vec!["roll", "pitch"]);
        assert!(px4_failure_bits(0).is_empty());
        assert_eq!(px4_nav_state_name(5), Some("Return"));
        assert_eq!(px4_nav_state_name(6), None);
    }
}
