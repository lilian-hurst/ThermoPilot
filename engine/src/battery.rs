//! Lecture de la batterie via IORegistry (AppleSmartBattery) — sans privilèges.

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use serde::Serialize;
use std::ffi::{c_char, c_void, CString};

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IORegistryEntryCreateCFProperties(entry: u32, props: *mut *mut c_void, alloc: *const c_void, opts: u32) -> i32;
    fn IOObjectRelease(obj: u32) -> i32;
}

type Dict = CFDictionary<CFString, CFType>;

#[derive(Clone, Serialize, Default)]
pub struct BatteryInfo {
    pub percent: f64,
    pub current_mah: f64,
    pub max_mah: f64,
    pub design_mah: f64,
    /// Santé « Apple » (Réglages > Batterie) = capacité nominale / capacité d'origine
    pub health_pct: f64,
    /// Santé brute = capacité max mesurée par la jauge / capacité d'origine
    pub raw_health_pct: f64,
    pub nominal_mah: f64,
    pub cycles: u32,
    pub design_cycles: u32,
    pub temperature_c: f64,
    pub voltage_v: f64,
    pub amperage_a: f64,
    /// Puissance batterie : > 0 en charge, < 0 en décharge
    pub power_w: f64,
    pub is_charging: bool,
    pub external_connected: bool,
    pub fully_charged: bool,
    /// Minutes avant vide (sur batterie) / avant la cible de charge (en charge)
    pub minutes_remaining: Option<f64>,
    pub adapter_watts: Option<f64>,
    pub adapter_name: Option<String>,
    pub not_charging_reason: i64,
    /// Puissance tirée de l'adaptateur par le système (secteur), W
    pub system_power_in_w: Option<f64>,
}

fn num(d: &Dict, k: &str) -> Option<i64> {
    d.find(CFString::new(k)).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64())
}
fn boolean(d: &Dict, k: &str) -> Option<bool> {
    d.find(CFString::new(k)).and_then(|v| v.downcast::<CFBoolean>()).map(bool::from)
}
fn string(d: &Dict, k: &str) -> Option<String> {
    d.find(CFString::new(k)).and_then(|v| v.downcast::<CFString>()).map(|s| s.to_string())
}
fn dict(d: &Dict, k: &str) -> Option<Dict> {
    let v = d.find(CFString::new(k))?;
    if v.instance_of::<CFDictionary>() {
        Some(unsafe { Dict::wrap_under_get_rule(v.as_CFTypeRef() as CFDictionaryRef) })
    } else {
        None
    }
}

pub fn read() -> Option<BatteryInfo> {
    let d: Dict = unsafe {
        let name = CString::new("AppleSmartBattery").unwrap();
        let svc = IOServiceGetMatchingService(0, IOServiceMatching(name.as_ptr()));
        if svc == 0 {
            return None;
        }
        let mut props: *mut c_void = std::ptr::null_mut();
        let kr = IORegistryEntryCreateCFProperties(svc, &mut props, std::ptr::null(), 0);
        IOObjectRelease(svc);
        if kr != 0 || props.is_null() {
            return None;
        }
        Dict::wrap_under_create_rule(props as CFDictionaryRef)
    };
    if boolean(&d, "BatteryInstalled") == Some(false) {
        return None;
    }
    let bd = dict(&d, "BatteryData");
    let pt = dict(&d, "PowerTelemetryData");
    let pick = |k: &str| {
        num(&d, k).or_else(|| bd.as_ref().and_then(|b| num(b, k))).or_else(|| pt.as_ref().and_then(|b| num(b, k)))
    };

    let current_mah = pick("AppleRawCurrentCapacity").unwrap_or(0) as f64;
    let max_mah = pick("AppleRawMaxCapacity").or_else(|| pick("NominalChargeCapacity")).unwrap_or(0) as f64;
    let nominal_mah = pick("NominalChargeCapacity").map(|v| v as f64).unwrap_or(max_mah);
    let design_mah = pick("DesignCapacity").unwrap_or(0) as f64;
    let percent = num(&d, "CurrentCapacity").unwrap_or(0) as f64;
    let voltage_v = num(&d, "Voltage").unwrap_or(0) as f64 / 1000.0;
    let amperage_a = num(&d, "InstantAmperage").filter(|v| *v != 0).or_else(|| num(&d, "Amperage")).unwrap_or(0) as f64 / 1000.0;
    let temp_of = |k: &str| {
        [Some(&d), bd.as_ref()].into_iter().flatten().filter_map(|x| num(x, k)).find(|v| *v > 0)
    };
    let temperature_c = temp_of("Temperature").or_else(|| temp_of("VirtualTemperature")).unwrap_or(0) as f64 / 100.0;
    let adapter = dict(&d, "AdapterDetails");
    let charger = dict(&d, "ChargerData");
    let external_connected = boolean(&d, "ExternalConnected").unwrap_or(false);
    let is_charging = boolean(&d, "IsCharging").unwrap_or(false);

    let valid = |v: Option<i64>| v.filter(|m| *m > 0 && *m < 65535).map(|m| m as f64);
    let minutes_remaining = if is_charging { valid(num(&d, "AvgTimeToFull")) } else if !external_connected { valid(num(&d, "AvgTimeToEmpty")) } else { None };

    Some(BatteryInfo {
        percent,
        current_mah,
        max_mah,
        design_mah,
        health_pct: if design_mah > 0.0 { (nominal_mah / design_mah * 100.0).round().min(100.0) } else { 0.0 },
        raw_health_pct: if design_mah > 0.0 { round1(max_mah / design_mah * 100.0) } else { 0.0 },
        nominal_mah,
        cycles: num(&d, "CycleCount").unwrap_or(0) as u32,
        design_cycles: num(&d, "DesignCycleCount9C").unwrap_or(1000) as u32,
        temperature_c: round1(temperature_c),
        voltage_v: (voltage_v * 100.0).round() / 100.0,
        amperage_a: (amperage_a * 100.0).round() / 100.0,
        power_w: round1(voltage_v * amperage_a),
        is_charging,
        external_connected,
        fully_charged: boolean(&d, "FullyCharged").unwrap_or(false),
        minutes_remaining,
        adapter_watts: adapter.as_ref().and_then(|a| num(a, "Watts")).filter(|_| external_connected).map(|w| w as f64),
        adapter_name: adapter.as_ref().and_then(|a| string(a, "Description").or_else(|| string(a, "Name"))).filter(|_| external_connected),
        system_power_in_w: pick("SystemPowerIn").filter(|_| external_connected).map(|mw| round1(mw as f64 / 1000.0)),
        not_charging_reason: charger.as_ref().and_then(|c| num(c, "NotChargingReason")).unwrap_or(0),
    })
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
