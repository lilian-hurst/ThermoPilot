//! Classification des capteurs SMC (Apple Silicon M3) + charge CPU/GPU.

use crate::smc::Smc;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct Sensor {
    pub key: String,
    pub group: &'static str,
    pub value: f64,
}

#[derive(Clone, Serialize)]
pub struct Group {
    pub id: &'static str,
    pub name: &'static str,
    pub avg: f64,
    pub max: f64,
    pub count: usize,
    /// Seuils indicatifs (°C) : au-delà de `warn` -> chaud, au-delà de `crit` -> critique
    pub warn: f64,
    pub crit: f64,
}

/// (id, nom, préfixes de clés, seuil warn, seuil crit)
const GROUPS: &[(&str, &str, &[&str], f64, f64)] = &[
    ("cpu_p", "CPU · cœurs performance", &["Tp"], 85.0, 98.0),
    ("cpu_e", "CPU · cœurs efficacité", &["Te"], 85.0, 98.0),
    ("gpu", "GPU", &["Tg"], 85.0, 98.0),
    ("soc", "SoC (die)", &["Ts0C", "Ts0D", "Ts0E", "Ts04", "Ts05", "Ts06", "Ts0R", "Ts0S", "Ts0T", "Ts0U", "Ts0V", "Ts0W", "Ts0h", "Ts0i"], 85.0, 98.0),
    ("ssd", "SSD (NAND)", &["TH0a", "TH0b", "TH0x"], 60.0, 75.0),
    ("battery", "Batterie", &["TB0T", "TB1T", "TB2T"], 40.0, 50.0),
    ("wifi", "Wi-Fi", &["TW0P"], 70.0, 85.0),
    ("palm", "Repose-poignets", &["Ts0P", "Ts1P"], 38.0, 45.0),
    ("air", "Flux d'air", &["TaLP", "TaRF", "TaTP", "TaLT", "TaRT", "TaLW", "TaRW"], 65.0, 80.0),
];

pub struct SensorMap {
    entries: Vec<(String, usize)>, // (clé, index du groupe)
    /// Sous-ensemble pour le mode arrière-plan : 1 capteur sur 4 dans les gros groupes (96 capteurs P-cores…)
    sparse: Vec<bool>,
}

impl SensorMap {
    /// Parcourt toutes les clés SMC une fois et retient les capteurs de température exploitables.
    pub fn discover(smc: &Smc) -> SensorMap {
        let mut entries = Vec::new();
        for key in smc.all_keys() {
            if !key.starts_with('T') || smc.type_of(&key).as_deref() != Some("flt ") {
                continue;
            }
            let gi = GROUPS.iter().position(|(_, _, pats, _, _)| {
                pats.iter().any(|p| if p.len() == 4 { key == *p } else { key.starts_with(p) })
            });
            if let Some(gi) = gi {
                if let Ok(v) = smc.read_f64(&key) {
                    if plausible(v) {
                        entries.push((key, gi));
                    }
                }
            }
        }
        let mut seen = vec![0usize; GROUPS.len()];
        let sizes: Vec<usize> = (0..GROUPS.len()).map(|g| entries.iter().filter(|e| e.1 == g).count()).collect();
        let sparse = entries
            .iter()
            .map(|(_, g)| {
                let i = seen[*g];
                seen[*g] += 1;
                sizes[*g] <= 12 || i % 4 == 0
            })
            .collect();
        SensorMap { entries, sparse }
    }

    /// Lecture allégée (app en arrière-plan) : suffit pour l'indicateur de la barre des menus.
    pub fn read_sparse(&self, smc: &Smc) -> (Vec<Sensor>, Vec<Group>) {
        self.read_filtered(smc, |i, _| self.sparse[i])
    }

    pub fn read(&self, smc: &Smc) -> (Vec<Sensor>, Vec<Group>) {
        self.read_only(smc, None)
    }

    /// `only` : ne lit que ces groupes (ex. la batterie quand personne ne regarde) — chaque clé SMC coûte ~0,2 ms.
    pub fn read_only(&self, smc: &Smc, only: Option<&[&str]>) -> (Vec<Sensor>, Vec<Group>) {
        self.read_filtered(smc, |_, gi| only.map_or(true, |o| o.contains(&GROUPS[gi].0)))
    }

    fn read_filtered(&self, smc: &Smc, keep: impl Fn(usize, usize) -> bool) -> (Vec<Sensor>, Vec<Group>) {
        let mut sensors = Vec::with_capacity(self.entries.len());
        let mut acc: Vec<Vec<f64>> = vec![Vec::new(); GROUPS.len()];
        for (i, (key, gi)) in self.entries.iter().enumerate() {
            if !keep(i, *gi) {
                continue;
            }
            if let Ok(v) = smc.read_f64(key) {
                if plausible(v) {
                    acc[*gi].push(v);
                    sensors.push(Sensor { key: key.clone(), group: GROUPS[*gi].0, value: round1(v) });
                }
            }
        }
        let groups = GROUPS
            .iter()
            .zip(acc)
            .filter(|(_, vals)| !vals.is_empty())
            .map(|((id, name, _, warn, crit), vals)| Group {
                id,
                name: if crate::i18n::en() { name_en(id) } else { name },
                avg: round1(vals.iter().sum::<f64>() / vals.len() as f64),
                max: round1(vals.iter().cloned().fold(f64::MIN, f64::max)),
                count: vals.len(),
                warn: *warn,
                crit: *crit,
            })
            .collect();
        (sensors, groups)
    }
}

fn name_en(id: &str) -> &'static str {
    match id {
        "cpu_p" => "CPU · performance cores",
        "cpu_e" => "CPU · efficiency cores",
        "gpu" => "GPU",
        "soc" => "SoC (die)",
        "ssd" => "SSD (NAND)",
        "battery" => "Battery",
        "wifi" => "Wi-Fi",
        "palm" => "Palm rest",
        "air" => "Airflow",
        _ => "Sensor",
    }
}

fn plausible(v: f64) -> bool {
    v.is_finite() && v > 5.0 && v < 130.0
}

pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

// ---------- Charge CPU (mach host_processor_info) ----------

extern "C" {
    fn mach_host_self() -> u32;
    fn host_processor_info(host: u32, flavor: i32, out_count: *mut u32, info: *mut *mut i32, info_count: *mut u32) -> i32;
    fn vm_deallocate(task: u32, addr: usize, size: usize) -> i32;
    static mach_task_self_: u32;
}

pub struct CpuLoad {
    prev: Option<Vec<[u64; 4]>>,
}

impl CpuLoad {
    pub fn new() -> Self {
        CpuLoad { prev: None }
    }

    fn ticks() -> Option<Vec<[u64; 4]>> {
        unsafe {
            let mut count = 0u32;
            let mut info: *mut i32 = std::ptr::null_mut();
            let mut info_count = 0u32;
            if host_processor_info(mach_host_self(), 2, &mut count, &mut info, &mut info_count) != 0 {
                return None;
            }
            let slice = std::slice::from_raw_parts(info, info_count as usize);
            let v = (0..count as usize)
                .map(|i| {
                    let s = &slice[i * 4..i * 4 + 4];
                    [s[0] as u32 as u64, s[1] as u32 as u64, s[2] as u32 as u64, s[3] as u32 as u64]
                })
                .collect();
            vm_deallocate(mach_task_self_, info as usize, info_count as usize * 4);
            Some(v)
        }
    }

    /// Retourne (charge totale %, charge par cœur %)
    pub fn sample(&mut self) -> (Option<f64>, Vec<f64>) {
        let now = match Self::ticks() {
            Some(t) => t,
            None => return (None, vec![]),
        };
        let res = match &self.prev {
            Some(prev) if prev.len() == now.len() => {
                let mut tot_busy = 0u64;
                let mut tot_all = 0u64;
                let per: Vec<f64> = now
                    .iter()
                    .zip(prev)
                    .map(|(n, p)| {
                        let d: Vec<u64> = (0..4).map(|i| n[i].wrapping_sub(p[i])).collect();
                        let busy = d[0] + d[1] + d[3];
                        let all = busy + d[2];
                        tot_busy += busy;
                        tot_all += all;
                        if all == 0 { 0.0 } else { round1(busy as f64 * 100.0 / all as f64) }
                    })
                    .collect();
                let total = if tot_all == 0 { 0.0 } else { round1(tot_busy as f64 * 100.0 / tot_all as f64) };
                (Some(total), per)
            }
            _ => (None, vec![]),
        };
        self.prev = Some(now);
        res
    }
}

// ---------- Charge GPU (IOAccelerator / PerformanceStatistics) ----------

mod gpu_ffi {
    use std::ffi::{c_char, c_void};
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        pub fn IOServiceGetMatchingServices(main_port: u32, matching: *mut c_void, iter: *mut u32) -> i32;
        pub fn IOIteratorNext(iter: u32) -> u32;
        pub fn IOObjectRelease(obj: u32) -> i32;
        pub fn IORegistryEntryCreateCFProperty(entry: u32, key: *const c_void, alloc: *const c_void, opts: u32) -> *const c_void;
    }
}

pub fn gpu_usage() -> Option<f64> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use gpu_ffi::*;
    unsafe {
        let name = std::ffi::CString::new("IOAccelerator").unwrap();
        let mut iter = 0u32;
        if IOServiceGetMatchingServices(0, IOServiceMatching(name.as_ptr()), &mut iter) != 0 {
            return None;
        }
        let mut result = None;
        loop {
            let entry = IOIteratorNext(iter);
            if entry == 0 {
                break;
            }
            let key = CFString::from_static_string("PerformanceStatistics");
            let prop = IORegistryEntryCreateCFProperty(entry, key.as_concrete_TypeRef() as *const _, std::ptr::null(), 0);
            IOObjectRelease(entry);
            if prop.is_null() {
                continue;
            }
            let dict: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_create_rule(prop as _);
            if let Some(v) = dict.find(CFString::from_static_string("Device Utilization %")) {
                if let Some(n) = v.downcast::<CFNumber>() {
                    result = n.to_f64();
                }
            }
            if result.is_some() {
                break;
            }
        }
        IOObjectRelease(iter);
        result
    }
}

pub fn cpu_model() -> String {
    unsafe {
        let name = std::ffi::CString::new("machdep.cpu.brand_string").unwrap();
        let mut buf = [0u8; 128];
        let mut len = buf.len();
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut(), 0) == 0 {
            let s = &buf[..len.saturating_sub(1)];
            return String::from_utf8_lossy(s).trim_matches(char::from(0)).to_string();
        }
    }
    "Apple Silicon".into()
}

pub fn hw_model() -> String {
    unsafe {
        let name = std::ffi::CString::new("hw.model").unwrap();
        let mut buf = [0u8; 64];
        let mut len = buf.len();
        if libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr() as *mut _, &mut len, std::ptr::null_mut(), 0) == 0 {
            return String::from_utf8_lossy(&buf[..len.saturating_sub(1)]).trim_matches(char::from(0)).to_string();
        }
    }
    String::new()
}
