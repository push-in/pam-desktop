//! Battery, connectivity and power-saving state as each operating system
//! reports them. Every probe is cheap enough to run on a polling thread.

pub const NETWORK_NONE: i64 = 1;
pub const NETWORK_WIFI: i64 = 2;
pub const NETWORK_CELLULAR: i64 = 3;
pub const NETWORK_ETHERNET: i64 = 4;
pub const NETWORK_OTHER: i64 = 5;

/// What the device reports right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    /// Charge in `0..=1`; `-1` when there is no battery or it is unknown.
    pub battery_level: f64,
    pub charging: bool,
    pub network: i64,
    /// The system is saving energy (battery saver, Low Power Mode).
    pub low_power: bool,
}

#[must_use]
pub fn status() -> Status {
    platform::status()
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;

    use super::{Status, NETWORK_ETHERNET, NETWORK_NONE, NETWORK_OTHER, NETWORK_WIFI};

    pub fn status() -> Status {
        let (battery_level, charging) = battery();
        Status { battery_level, charging, network: network(), low_power: low_power() }
    }

    fn battery() -> (f64, bool) {
        let Ok(entries) = fs::read_dir("/sys/class/power_supply") else { return (-1.0, false) };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if fs::read_to_string(path.join("type")).unwrap_or_default().trim() != "Battery" {
                continue;
            }
            // Peripherals (mice, headsets) report batteries too.
            if fs::read_to_string(path.join("scope")).is_ok_and(|scope| scope.trim() == "Device") {
                continue;
            }
            let Some(capacity) = fs::read_to_string(path.join("capacity")).ok().and_then(|value| value.trim().parse::<f64>().ok()) else {
                continue;
            };
            let status = fs::read_to_string(path.join("status")).unwrap_or_default();
            return ((capacity / 100.0).clamp(0.0, 1.0), matches!(status.trim(), "Charging" | "Full"));
        }
        (-1.0, false)
    }

    fn network() -> i64 {
        let Ok(entries) = fs::read_dir("/sys/class/net") else { return NETWORK_OTHER };
        let mut connected = NETWORK_NONE;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if entry.file_name() == "lo" || !fs::read_to_string(path.join("operstate")).is_ok_and(|state| state.trim() == "up") {
                continue;
            }
            if path.join("wireless").exists() {
                connected = connected.max(NETWORK_WIFI);
            } else if path.join("device").exists() {
                return NETWORK_ETHERNET;
            }
        }
        connected
    }

    /// The firmware power profile, where the machine exposes one.
    fn low_power() -> bool {
        fs::read_to_string("/sys/firmware/acpi/platform_profile").is_ok_and(|profile| matches!(profile.trim(), "low-power" | "quiet"))
    }
}

#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(any(target_os = "macos", test))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod macos {
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::{Status, NETWORK_ETHERNET, NETWORK_NONE, NETWORK_WIFI};

    /// The system tools are asked at most this often.
    const FRESH: Duration = Duration::from_secs(10);

    static KNOWN: Mutex<Option<(Instant, Status)>> = Mutex::new(None);

    pub fn status() -> Status {
        let mut known = KNOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((asked, status)) = *known
            && asked.elapsed() < FRESH
        {
            return status;
        }
        let (battery_level, charging) = parse_battery(&output("/usr/bin/pmset", &["-g", "batt"]));
        let status = Status {
            battery_level,
            charging,
            network: network(),
            low_power: parse_low_power(&output("/usr/bin/pmset", &["-g"])),
        };
        *known = Some((Instant::now(), status));
        status
    }

    fn output(program: &str, arguments: &[&str]) -> String {
        Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
    }

    /// `pmset -g batt`: ` -InternalBattery-0 (id=…) 85%; charging; 1:02 remaining`.
    pub(super) fn parse_battery(text: &str) -> (f64, bool) {
        for line in text.lines().filter(|line| line.contains("InternalBattery")) {
            let Some(end) = line.find('%') else { continue };
            let digits = line[..end].rsplit(|character: char| !character.is_ascii_digit()).next().unwrap_or_default();
            let Ok(percent) = digits.parse::<f64>() else { continue };
            let state = line[end..].split(';').nth(1).map(str::trim).unwrap_or_default();
            return ((percent / 100.0).clamp(0.0, 1.0), matches!(state, "charging" | "charged" | "finishing charge"));
        }
        (-1.0, false)
    }

    /// `pmset -g`: ` lowpowermode         1`.
    pub(super) fn parse_low_power(text: &str) -> bool {
        text.lines().any(|line| {
            let mut words = line.split_whitespace();
            words.next() == Some("lowpowermode") && words.next() == Some("1")
        })
    }

    fn network() -> i64 {
        let route = output("/sbin/route", &["-n", "get", "default"]);
        let Some(interface) = route.lines().find_map(|line| line.trim().strip_prefix("interface:")).map(str::trim) else {
            return NETWORK_NONE;
        };
        if is_wifi(&output("/usr/sbin/networksetup", &["-listallhardwareports"]), interface) {
            NETWORK_WIFI
        } else {
            NETWORK_ETHERNET
        }
    }

    /// `networksetup -listallhardwareports`: `Hardware Port: Wi-Fi` followed by `Device: en0`.
    pub(super) fn is_wifi(ports: &str, interface: &str) -> bool {
        let mut port = "";
        for line in ports.lines() {
            if let Some(name) = line.strip_prefix("Hardware Port:") {
                port = name.trim();
            } else if line.strip_prefix("Device:").is_some_and(|device| device.trim() == interface) {
                return port.contains("Wi-Fi") || port.contains("AirPort");
            }
        }
        false
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST,
        IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    use super::{Status, NETWORK_CELLULAR, NETWORK_ETHERNET, NETWORK_NONE, NETWORK_OTHER, NETWORK_WIFI};

    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    const AF_UNSPEC: u32 = 0;
    const OPER_STATUS_UP: i32 = 1;
    const IF_TYPE_ETHERNET: u32 = 6;
    const IF_TYPE_LOOPBACK: u32 = 24;
    const IF_TYPE_WIFI: u32 = 71;
    const IF_TYPE_WWAN: u32 = 243;
    const IF_TYPE_WWAN2: u32 = 244;
    /// `BatteryFlag`: charging, and no system battery.
    const BATTERY_CHARGING: u8 = 8;
    const BATTERY_ABSENT: u8 = 128;
    const BATTERY_UNKNOWN: u8 = 255;

    pub fn status() -> Status {
        // SAFETY: an all-zero SYSTEM_POWER_STATUS is a valid value of this
        // plain structure, and the call only writes into it.
        let power = unsafe {
            let mut power: SYSTEM_POWER_STATUS = std::mem::zeroed();
            (GetSystemPowerStatus(&mut power) != 0).then_some(power)
        };
        let (battery_level, charging, low_power) = match power {
            Some(power) => {
                let present = power.BatteryFlag != BATTERY_UNKNOWN && power.BatteryFlag & BATTERY_ABSENT == 0 && power.BatteryLifePercent <= 100;
                let level = if present { f64::from(power.BatteryLifePercent) / 100.0 } else { -1.0 };
                let charging = present && (power.BatteryFlag & BATTERY_CHARGING != 0 || (power.ACLineStatus == 1 && power.BatteryLifePercent == 100));
                (level, charging, power.SystemStatusFlag == 1)
            }
            None => (-1.0, false, false),
        };
        Status { battery_level, charging, network: network(), low_power }
    }

    /// The best connected adapter that has a gateway.
    fn network() -> i64 {
        let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 16 * 1024;
        for _ in 0..3 {
            // 8-byte words keep the buffer aligned for the adapter records.
            let mut buffer = vec![0_u64; (size as usize).div_ceil(8)];
            // SAFETY: `buffer` holds at least `size` writable bytes, aligned
            // for IP_ADAPTER_ADDRESSES_LH; the list the call writes lives
            // inside it and is only read while `buffer` is alive.
            unsafe {
                let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
                match GetAdaptersAddresses(AF_UNSPEC, flags, std::ptr::null(), first, &mut size) {
                    0 => {
                        let mut best = NETWORK_NONE;
                        let mut adapter = first.cast_const();
                        while let Some(current) = adapter.as_ref() {
                            adapter = current.Next;
                            if current.OperStatus != OPER_STATUS_UP || current.IfType == IF_TYPE_LOOPBACK || current.FirstGatewayAddress.is_null() {
                                continue;
                            }
                            match current.IfType {
                                IF_TYPE_ETHERNET => return NETWORK_ETHERNET,
                                IF_TYPE_WIFI => best = NETWORK_WIFI,
                                IF_TYPE_WWAN | IF_TYPE_WWAN2 if best != NETWORK_WIFI => best = NETWORK_CELLULAR,
                                _ if best == NETWORK_NONE => best = NETWORK_OTHER,
                                _ => {}
                            }
                        }
                        return best;
                    }
                    ERROR_BUFFER_OVERFLOW => {}
                    _ => return NETWORK_OTHER,
                }
            }
        }
        NETWORK_OTHER
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{Status, NETWORK_OTHER};

    pub fn status() -> Status {
        Status { battery_level: -1.0, charging: false, network: NETWORK_OTHER, low_power: false }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn reports_a_coherent_status() {
        let status = status();
        assert!(status.battery_level == -1.0 || (0.0..=1.0).contains(&status.battery_level));
        assert!((NETWORK_NONE..=NETWORK_OTHER).contains(&status.network));
        let _ = NETWORK_CELLULAR;
    }

    #[test]
    fn reads_the_macos_power_report() {
        let charging = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=4653155)\t85%; charging; 1:02 remaining present: true\n";
        assert_eq!(macos::parse_battery(charging), (0.85, true));
        let draining = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t7%; discharging; 0:21 remaining present: true\n";
        assert_eq!(macos::parse_battery(draining), (0.07, false));
        let full = " -InternalBattery-0 (id=1)\t100%; charged; 0:00 remaining present: true";
        assert_eq!(macos::parse_battery(full), (1.0, true));
        assert_eq!(macos::parse_battery("Now drawing from 'AC Power'\n"), (-1.0, false));

        assert!(macos::parse_low_power("Battery Power:\n lowpowermode         1\n sleep                1\n"));
        assert!(!macos::parse_low_power(" lowpowermode         0\n"));
        assert!(!macos::parse_low_power(" sleep 1\n"));
    }

    #[test]
    fn tells_wifi_from_wired_ports_on_macos() {
        let ports = "Hardware Port: Ethernet Adapter (en4)\nDevice: en4\nEthernet Address: aa\n\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: bb\n\nHardware Port: Thunderbolt Bridge\nDevice: bridge0\n";
        assert!(macos::is_wifi(ports, "en0"));
        assert!(!macos::is_wifi(ports, "en4"));
        assert!(!macos::is_wifi(ports, "utun3"));
    }
}
