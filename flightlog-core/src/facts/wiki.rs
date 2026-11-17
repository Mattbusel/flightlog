//! The ArduPilot wiki statements each check rests on, quoted verbatim from
//! the wiki source (ArduPilot/ardupilot_wiki at [`WIKI_COMMIT`]).

/// ardupilot_wiki commit the quotes were checked against (2026-11-17).
pub const WIKI_COMMIT: &str = "3217b658a9fa3bb5c766d320eac5b447651b48f9";

/// A wiki page and the sentence a threshold comes from.
#[derive(Debug, Clone, Copy)]
pub struct Cite {
    /// Published page.
    pub url: &'static str,
    /// Source file in ArduPilot/ardupilot_wiki.
    pub file: &'static str,
    /// Verbatim sentence.
    pub quote: &'static str,
}

/// Vibration level thresholds (30 / 60 m/s/s).
pub const VIBE_LEVELS: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-measuring-vibration.html",
    file: "common/source/docs/common-measuring-vibration.rst",
    quote: "Vibration levels below 30m/s/s are normally acceptable.  Levels above 30m/s/s *may* have problems and levels above 60m/s/s nearly always have problems with position or altitude hold.",
};

/// "Mostly below 30m/s/s".
pub const VIBE_MOSTLY: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-measuring-vibration.html",
    file: "common/source/docs/common-measuring-vibration.rst",
    quote: "Check the recorded Vibe levels are mostly below 30m/s/s",
};

/// Clipping counts.
pub const VIBE_CLIP: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-measuring-vibration.html",
    file: "common/source/docs/common-measuring-vibration.rst",
    quote: "Ideally these should be zero for the whole flight but low numbers (<100) are likely ok especially if they occur during hard landings.  A regularly increasing number through the log indicates a serious vibration problem",
};

/// Vibration failsafe trigger.
pub const VIBE_FAILSAFE: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/vibration-failsafe.html",
    file: "copter/source/docs/vibration-failsafe.rst",
    quote: "The vibration failsafe will trigger if all of the following are true for at least one second:",
};

/// EKF failsafe rule.
pub const EKF_FAILSAFE: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-ekf-inav-failsafe.html",
    file: "common/source/docs/common-ekf-inav-failsafe.rst",
    quote: "The EKF failsafe will trigger when any two of the EKF \"variances\" for compass, position or velocity are higher than the :ref:`FS_EKF_THRESH <FS_EKF_THRESH>` parameter value for 1 second.",
};

/// FS_EKF_THRESH value 0.8.
pub const EKF_DEFAULT: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-ekf-inav-failsafe.html",
    file: "common/source/docs/common-ekf-inav-failsafe.rst",
    quote: "The vehicle switches to Land mode soon after both velocity and position innovations climb over the :ref:`FS_EKF_THRESH<FS_EKF_THRESH>` value of 0.8",
};

/// GPS HDop and NSats in DataFlash logs.
pub const GPS_DF: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-diagnosing-problems-using-logs.html",
    file: "common/source/docs/common-diagnosing-problems-using-logs.rst",
    quote: "Hdop values below 1.5 are very good, values over 2.0 could indicate the GPS positions are not good.  The number of satellites falling below 12 is also bad.",
};

/// GPS eph and satellites in tlogs.
pub const GPS_TLOG: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-diagnosing-problems-using-logs.html",
    file: "common/source/docs/common-diagnosing-problems-using-logs.rst",
    quote: "If using tlogs graph the GPS_RAW_IT group's \"eph\" and \"satellites_visible\" values.  Hdop values below 150 are good, values over 200 could indicate a bad position.",
};

/// ERR subsystem and code table.
pub const ERR_TABLE: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-diagnosing-problems-using-logs.html",
    file: "common/source/docs/common-diagnosing-problems-using-logs.rst",
    quote: "The Subsys (aka Sub-system) gives the area that generated the error and the ECode (aka Error Code) tells you what the error was specifically.",
};

/// Battery voltage failsafe.
pub const BATTERY: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/failsafe-battery.html",
    file: "copter/source/docs/failsafe-battery.rst",
    quote: "voltage drops below the voltage held in the :ref:`BATT_LOW_VOLT <BATT_LOW_VOLT>` parameter (or FS_BATT_VOLTAGE in older versions) for more than 10 seconds. The default voltage is 10.5 volts. If set to zero, the voltage based trigger will be disabled.",
};

/// Sag-compensated voltage option.
pub const BATTERY_SAG: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/failsafe-battery.html",
    file: "copter/source/docs/failsafe-battery.rst",
    quote: "- :ref:`BATT_FS_VOLTSRC <BATT_FS_VOLTSRC>` allows configuring whether the raw battery voltage or a sag corrected voltage is used",
};

/// Board voltage variation.
pub const BOARD_VCC: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-diagnosing-problems-using-logs.html",
    file: "common/source/docs/common-diagnosing-problems-using-logs.rst",
    quote: "Changes in the board voltage can also be a sign of a power problem.  Variations of 0.10 to 0.15 volts are normal.  Beyond that could be a sign that other devices sharing the power source with the autopilot are causing ripples in the power supply that could lead to a brown-out.",
};

/// Brown-outs end the log in the air.
pub const BROWNOUT: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/common-diagnosing-problems-using-logs.html",
    file: "common/source/docs/common-diagnosing-problems-using-logs.rst",
    quote: "They can normally be recognised by the logs suddenly ending while the vehicle is still in the air (i.e. barometer or EKF altitude is still reporting the vehicle's altitude is well above zero).",
};

/// Radio failsafe trigger.
pub const RADIO_FAILSAFE: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/radio-failsafe.html",
    file: "copter/source/docs/radio-failsafe.rst",
    quote: "If enabled and set-up correctly the radio failsafe will trigger if any of these conditions occur for more than :ref:`RC_FS_TIMEOUT<RC_FS_TIMEOUT>` seconds (default = 1 sec):",
};

/// GCS failsafe timeout.
pub const GCS_FAILSAFE: Cite = Cite {
    url: "https://ardupilot.org/copter/docs/gcs-failsafe.html",
    file: "copter/source/docs/gcs-failsafe.rst",
    quote: "If no heartbeat is received :ref:`FS_GCS_TIMEOUT<FS_GCS_TIMEOUT>` seconds (Default is 5 seconds), the GCS failsafe event will trigger based on your parameter settings.",
};

/// ERR subsystem names from the wiki's error table.
pub fn err_subsystem(s: i64) -> &'static str {
    match s {
        2 => "Radio",
        3 => "Compass",
        5 => "Radio Failsafe",
        6 => "Battery Failsafe",
        8 => "GCS Failsafe",
        9 => "Fence Failsafe",
        10 => "Flight mode Change failure",
        11 => "GPS",
        12 => "Crash Check",
        13 => "Flip mode",
        15 => "Parachute",
        16 => "EKF Check",
        17 => "EKF Failsafe",
        18 => "Barometer",
        19 => "CPU Load Watchdog",
        20 => "ADSB Failsafe",
        21 => "Terrain Data",
        22 => "Navigation",
        23 => "Terrain Failsafe",
        24 => "EKF Primary changed",
        25 => "Thrust Loss Check",
        26 => "Sensor Failsafe (Sub)",
        27 => "Leak Failsafe (Sub)",
        28 => "Pilot Input Timeout Failsafe (Sub only)",
        29 => "Vibration Failsafe",
        _ => "unknown",
    }
}

/// ERR code meaning from the wiki's error table, when it lists one.
pub fn err_code(s: i64, c: i64) -> Option<&'static str> {
    Some(match (s, c) {
        (2, 0) | (3, 0) | (18, 0) => "Errors Resolved",
        (2, 2) => "Late Frame : no updates received from receiver for two seconds",
        (3, 1) => "Failed to initialise (probably a hardware issue)",
        (3, 4) | (18, 4) => "Unhealthy : failed to read from the sensor",
        (5 | 6 | 8 | 17 | 19 | 23, 0) => "Failsafe Resolved",
        (5 | 6 | 8 | 17, 1) => "Failsafe Triggered",
        (9, 0) => "Failsafe Resolved",
        (9, 1) => "Altitude fence breach, Failsafe Triggered",
        (9, 2) => "Circular fence breach, Failsafe Triggered",
        (9, 3) => "Both Alt and Circular fence breached, Failsafe Triggered",
        (9, 4) => "Polygon fence breached, Failsafe Triggered",
        (11, 0) => "Glitch cleared",
        (11, 2) => "GPS Glitch occurred",
        (12, 1) => "Crash into ground detected.  Normally vehicle is disarmed soon after",
        (12, 2) => "Loss of control detected.  Normally parachute is released soon after",
        (13, 2) => "Flip abandoned (not armed, pilot input or timeout)",
        (15, 2) => "Not Deployed, vehicle too low",
        (15, 3) => "Not Deployed, vehicle landed",
        (16, 0) => "Variance cleared (position estimate OK)",
        (16, 2) => "Bad Variance (position estimate bad)",
        (19, 1) => "Failsafe Triggered (normally vehicle disarms)",
        (20, 0) => "Failsafe Resolved",
        (20, 1) => "No action just report to Pilot",
        (20, 2) => "Vehicle avoids by climbing or descending",
        (20, 3) => "Vehicle avoids by moving horizontally",
        (20, 4) => "Vehicle avoids by moving perpendicular to other vehicle",
        (20, 5) => "RTL invoked",
        (21, 2) => "missing terrain data",
        (22, 2) => "Failed to set destination",
        (22, 3) => "RTL restarted",
        (22, 4) => "Circle initialisation failed",
        (22, 5) => "Destination outside fence",
        (23, 1) => "Failsafe Triggered (normally vehicle RTLs)",
        (24, 0) => "1st EKF has become primary",
        (24, 1) => "2nd EKF has become primary",
        (25, 0) => "Thrust Restored",
        (25, 1) => "Thrust Loss Detected (altitude may be prioritised over yaw control)",
        (26, 0) => "Sensor Failsafe Cleared",
        (26, 1) => "Sensor Failsafe Triggered",
        (27, 0) => "Leak Failsafe Cleared",
        (27, 1) => "Leak Detector Failsafe Triggered",
        (28, 0) => "Pilot Input Failsafe Cleared",
        (28, 1) => "Pilot Input Failsafe Triggered",
        (29, 0) => "Excessive Vibration Compensation De-activated",
        (29, 1) => "Excessive Vibration Compenstaion Activated",
        _ => return None,
    })
}
