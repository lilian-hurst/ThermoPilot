//! Consommation énergétique par processus (proc_pid_rusage v6 : ri_energy_nj).

use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

extern "C" {
    fn proc_listallpids(buffer: *mut i32, buffersize: i32) -> i32;
    fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
    fn proc_name(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
    fn mach_timebase_info(info: *mut [u32; 2]) -> i32;
}

const RUSAGE_INFO_V6: i32 = 6;
// rusage_info_v6 : uuid (16 o = 2 u64) puis 56 champs u64
const WORDS: usize = 2 + 56;
const IDX_USER: usize = 2;
const IDX_SYSTEM: usize = 3;
const IDX_ENERGY_NJ: usize = 2 + 40;

#[derive(Clone, Serialize)]
pub struct ProcEnergy {
    pub name: String,
    pub pid: i32,
    pub count: usize,
    pub watts: f64,
    pub cpu_pct: f64,
}

/// Tous les PID du système.
pub fn all_pids() -> Vec<i32> {
    let mut pids = vec![0i32; 8192];
    let n = unsafe { proc_listallpids(pids.as_mut_ptr(), (pids.len() * 4) as i32) };
    pids.truncate(n.max(0) as usize);
    pids.retain(|p| *p > 0);
    pids
}

pub fn pid_name(pid: i32) -> Option<String> {
    let mut name_buf = [0u8; 256];
    let len = unsafe { proc_name(pid, name_buf.as_mut_ptr(), name_buf.len() as u32) };
    (len > 0).then(|| String::from_utf8_lossy(&name_buf[..len as usize]).to_string())
}

/// Ligne de commande complète (sysctl KERN_PROCARGS2), sans lancer de processus.
pub fn pid_args(pid: i32) -> Option<String> {
    let mut mib = [libc::CTL_KERN, 49 /* KERN_PROCARGS2 */, pid];
    let mut size: libc::size_t = 0;
    unsafe {
        if libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 || size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size];
        if libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr() as *mut _, &mut size, std::ptr::null_mut(), 0) != 0 {
            return None;
        }
        buf.truncate(size);
        let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
        let rest = &buf[4..];
        // chemin de l'exécutable, puis des NUL, puis argv
        let mut parts = rest.split(|b| *b == 0).filter(|p| !p.is_empty()).skip(1).take(argc);
        let mut out = String::new();
        for p in parts.by_ref() {
            out.push_str(&String::from_utf8_lossy(p));
            out.push(' ');
        }
        Some(out)
    }
}

pub struct EnergyTracker {
    prev: HashMap<i32, (u64, u64)>,
    last: Option<Instant>,
    numer: u64,
    denom: u64,
    pub top: Vec<ProcEnergy>,
    /// Dernier intervalle : (nom, joules, secondes CPU) pour tous les processus actifs
    pub deltas: Vec<(String, f64, f64)>,
    /// Durée du dernier intervalle (s)
    pub last_dt: f64,
}

impl EnergyTracker {
    pub fn new() -> Self {
        let mut tb = [0u32; 2];
        unsafe { mach_timebase_info(&mut tb) };
        EnergyTracker { prev: HashMap::new(), last: None, numer: tb[0].max(1) as u64, denom: tb[1].max(1) as u64, top: vec![], deltas: vec![], last_dt: 0.0 }
    }

    pub fn sample(&mut self) {
        let now = Instant::now();
        let pids = all_pids();
        if pids.is_empty() {
            return;
        }
        let dt = self.last.map(|l| now.duration_since(l).as_secs_f64()).unwrap_or(0.0);
        let mut current = HashMap::with_capacity(pids.len());
        let mut groups: HashMap<String, ProcEnergy> = HashMap::new();
        let mut buf = [0u64; WORDS];
        for pid in pids {
            if pid <= 0 || unsafe { proc_pid_rusage(pid, RUSAGE_INFO_V6, buf.as_mut_ptr()) } != 0 {
                continue;
            }
            let energy = buf[IDX_ENERGY_NJ];
            let cpu_ns = (buf[IDX_USER] + buf[IDX_SYSTEM]) as u128 * self.numer as u128 / self.denom as u128;
            let cpu_ns = cpu_ns as u64;
            current.insert(pid, (energy, cpu_ns));
            if dt <= 0.0 {
                continue;
            }
            let Some(&(pe, pc)) = self.prev.get(&pid) else { continue };
            let watts = energy.saturating_sub(pe) as f64 / 1e9 / dt;
            let cpu = cpu_ns.saturating_sub(pc) as f64 / 1e9 / dt * 100.0;
            if watts < 0.01 && cpu < 0.5 {
                continue;
            }
            let name = pid_name(pid).unwrap_or_else(|| format!("pid {pid}"));
            let g = groups.entry(name.clone()).or_insert(ProcEnergy { name, pid, count: 0, watts: 0.0, cpu_pct: 0.0 });
            if watts > 0.0 && g.watts < watts {
                g.pid = pid;
            }
            g.count += 1;
            g.watts += watts;
            g.cpu_pct += cpu;
        }
        if dt > 0.0 {
            self.deltas = groups.values().map(|g| (g.name.clone(), g.watts * dt, g.cpu_pct / 100.0 * dt)).collect();
            self.last_dt = dt;
            let mut v: Vec<ProcEnergy> = groups.into_values().collect();
            v.sort_by(|a, b| b.watts.partial_cmp(&a.watts).unwrap().then(b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap()));
            v.truncate(10);
            for p in &mut v {
                p.watts = (p.watts * 100.0).round() / 100.0;
                p.cpu_pct = (p.cpu_pct * 10.0).round() / 10.0;
            }
            self.top = v;
        }
        self.prev = current;
        self.last = Some(now);
    }
}
