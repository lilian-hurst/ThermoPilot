//! Boucle d'échantillonnage + application des consignes ventilateurs.

use crate::battery::{self, BatteryInfo};
use crate::battery_advice::{self, Tip};
use crate::charge::{ChargeConfig, ChargeStatus, Limiter};
use crate::control::{self, Advice, Mode, SAFETY_OFF, SAFETY_ON};
use crate::energy::{EnergyTracker, ProcEnergy};
use crate::usage::Recorder;
use crate::fans::{self, Fan};
use crate::power::{self, PmsetProfiles};
use serde::Deserialize;
use std::collections::BTreeMap;
use crate::sensors::{self, CpuLoad, Group, Sensor, SensorMap};
use crate::smc::Smc;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub static STOP: AtomicBool = AtomicBool::new(false);

/// Délai de grâce après la déconnexion du dernier client (couvre une reconnexion de l'app)
const CLIENT_GRACE: Duration = Duration::from_secs(5);

#[derive(Serialize)]
pub struct Snapshot {
    pub timestamp: f64,
    pub privileged: bool,
    pub cpu_model: String,
    pub hw_model: String,
    pub mode: Mode,
    pub fans: Vec<Fan>,
    pub groups: Vec<Group>,
    pub sensors: Vec<Sensor>,
    pub cpu_usage: Option<f64>,
    pub cpu_cores: Vec<f64>,
    pub gpu_usage: Option<f64>,
    pub power_w: Option<f64>,
    pub hotspot: f64,
    pub trend_c_per_min: f64,
    pub safety_override: bool,
    pub advice: Advice,
    pub last_error: Option<String>,
    pub battery: Option<BatteryInfo>,
    pub charge: ChargeStatus,
    pub power_settings: Option<PmsetProfiles>,
    pub pmset_backup: Option<BTreeMap<String, i64>>,
    pub top_processes: Vec<ProcEnergy>,
    pub battery_tips: Vec<Tip>,
    pub conflicts: Vec<String>,
}

const SETTINGS_PATH: &str = "/Library/Application Support/ThermoPilot/settings.json";

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Settings {
    #[serde(default)]
    pub charge: ChargeConfig,
    #[serde(default)]
    pub pmset_backup: Option<BTreeMap<String, i64>>,
}

pub struct Shared {
    pub privileged: bool,
    pub mode: Mutex<Mode>,
    pub snapshot: Mutex<String>,
    pub settings: Mutex<Settings>,
    pub pmset_dirty: AtomicBool,
    pub woke: AtomicBool,
    /// Nombre de clients (l'app) connectés au socket
    pub clients: AtomicUsize,
    /// Moment où le dernier client s'est déconnecté
    pub last_client_left: Mutex<Option<Instant>>,
    /// Rapport d'utilisation batterie (JSON), calculé seulement quand l'app l'affiche
    pub usage: Mutex<String>,
    /// Dernière requête « status » (UI visible => cadence 1 s) et « usage »
    pub last_status: Mutex<Option<Instant>>,
    pub last_usage_req: Mutex<Option<Instant>>,
    status_gap: Mutex<Option<Duration>>,
    wake: (Mutex<bool>, Condvar),
}

impl Shared {
    pub fn new(privileged: bool) -> Arc<Shared> {
        Arc::new(Shared {
            privileged,
            mode: Mutex::new(Mode::Auto),
            snapshot: Mutex::new("null".into()),
            settings: Mutex::new(if privileged { Self::load() } else { Settings::default() }),
            pmset_dirty: AtomicBool::new(true),
            woke: AtomicBool::new(false),
            clients: AtomicUsize::new(0),
            last_client_left: Mutex::new(None),
            usage: Mutex::new("null".into()),
            last_status: Mutex::new(None),
            last_usage_req: Mutex::new(None),
            status_gap: Mutex::new(None),
            wake: (Mutex::new(false), Condvar::new()),
        })
    }

    fn load() -> Settings {
        std::fs::read_to_string(SETTINGS_PATH).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if !self.privileged {
            return;
        }
        let s = self.settings.lock().unwrap().clone();
        if let Some(dir) = std::path::Path::new(SETTINGS_PATH).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(SETTINGS_PATH, json);
        }
    }

    /// L'app interroge au moins chaque seconde : une fenêtre est visible.
    /// (deux requêtes « status » à moins de 2 s d'intervalle ; en arrière-plan l'app interroge toutes les 5 s)
    pub fn ui_active(&self) -> bool {
        let last = *self.last_status.lock().unwrap();
        let gap = *self.status_gap.lock().unwrap();
        last.map_or(false, |t| t.elapsed() < Duration::from_millis(2500)) && gap.map_or(false, |g| g < Duration::from_secs(2))
    }

    /// À appeler à chaque requête « status ».
    pub fn note_status(&self) {
        let now = Instant::now();
        let prev = self.last_status.lock().unwrap().replace(now);
        *self.status_gap.lock().unwrap() = prev.map(|p| now.duration_since(p));
    }

    pub fn usage_wanted(&self) -> bool {
        self.last_usage_req.lock().unwrap().map_or(false, |t| t.elapsed() < Duration::from_secs(90))
    }

    pub fn has_clients(&self) -> bool {
        self.clients.load(Ordering::SeqCst) > 0
    }

    pub fn wake_now(&self) {
        let (lock, cv) = &self.wake;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }

    pub fn set_mode(&self, m: Mode) {
        *self.mode.lock().unwrap() = m;
        let (lock, cv) = &self.wake;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }

    fn sleep(&self, d: Duration) {
        let (lock, cv) = &self.wake;
        let g = lock.lock().unwrap();
        let (mut g, _) = cv.wait_timeout_while(g, d, |woken| !*woken).unwrap();
        *g = false;
    }
}

pub struct Engine {
    smc: Arc<Smc>,
    limiter: Limiter,
    energy: EnergyTracker,
    pub usage: Recorder,
    pmset: Option<PmsetProfiles>,
    pmset_at: Option<Instant>,
    conflicts: Vec<String>,
    conflicts_at: Option<Instant>,
    energy_at: Option<Instant>,
    usage_at: Option<Instant>,
    fan_limits: Vec<(f64, f64)>,
    last_tick: Option<Instant>,
    /// CLI (status, battery…) : toujours tout lire
    pub always_full: bool,
    ticks: u64,
    power_ema: Option<f64>,
    map: SensorMap,
    cpu: CpuLoad,
    cpu_model: String,
    hw_model: String,
    history: VecDeque<(Instant, f64)>,
    smoothed: Option<f64>,
    commanded: Vec<f64>,
    applied_manual: bool,
    safety: bool,
    last_error: Option<String>,
}

impl Engine {
    pub fn new(smc: Smc) -> Engine {
        let map = SensorMap::discover(&smc);
        let mut cpu = CpuLoad::new();
        cpu.sample();
        let limiter = Limiter::new(&smc);
        let fan_limits = fans::limits(&smc);
        let mut energy = EnergyTracker::new();
        energy.sample();
        Engine {
            smc: Arc::new(smc),
            limiter,
            energy,
            usage: Recorder::new(),
            pmset: None,
            pmset_at: None,
            conflicts: vec![],
            conflicts_at: None,
            energy_at: None,
            usage_at: None,
            fan_limits,
            last_tick: None,
            always_full: false,
            ticks: 0,
            power_ema: None,
            map,
            cpu,
            cpu_model: sensors::cpu_model(),
            hw_model: sensors::hw_model(),
            history: VecDeque::new(),
            smoothed: None,
            commanded: vec![],
            applied_manual: false,
            safety: false,
            last_error: None,
        }
    }

    fn trend(&self) -> f64 {
        // Régression linéaire sur les ~30 dernières secondes -> °C/min
        let n = self.history.len();
        if n < 5 {
            return 0.0;
        }
        let t0 = self.history[0].0;
        let xs: Vec<f64> = self.history.iter().map(|(t, _)| t.duration_since(t0).as_secs_f64()).collect();
        let ys: Vec<f64> = self.history.iter().map(|(_, v)| *v).collect();
        let mx = xs.iter().sum::<f64>() / n as f64;
        let my = ys.iter().sum::<f64>() / n as f64;
        let (mut num, mut den) = (0.0, 0.0);
        for i in 0..n {
            num += (xs[i] - mx) * (ys[i] - my);
            den += (xs[i] - mx).powi(2);
        }
        if den == 0.0 { 0.0 } else { num / den * 60.0 }
    }

    pub fn tick(&mut self, shared: &Shared) -> Snapshot {
        // App fermée (plus aucun client depuis 5 s) : on rend les ventilateurs à macOS
        if shared.clients.load(Ordering::SeqCst) == 0 && *shared.mode.lock().unwrap() != Mode::Auto {
            let left = *shared.last_client_left.lock().unwrap();
            if left.map_or(true, |t| t.elapsed() >= CLIENT_GRACE) {
                eprintln!("fanengine: plus aucun client connecté, ventilateurs rendus à macOS");
                *shared.mode.lock().unwrap() = Mode::Auto;
            }
        }
        let mode = shared.mode.lock().unwrap().clone();
        let now = Instant::now();
        let dt = self.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(1.0);
        self.last_tick = Some(now);
        // Personne ne regarde et macOS pilote les ventilateurs : on ne lit que le strict nécessaire
        let watched = self.always_full || shared.has_clients() || mode != Mode::Auto || self.applied_manual;
        let mut fan_list = if watched { fans::read_with(&self.smc, &self.fan_limits) } else { vec![] };
        let full = self.always_full || mode != Mode::Auto || self.applied_manual || shared.ui_active();
        let (sensors, groups) = if full {
            self.map.read(&self.smc)
        } else if watched {
            self.map.read_sparse(&self.smc) // app en arrière-plan
        } else {
            self.map.read_only(&self.smc, Some(&["battery"])) // app fermée
        };
        let (cpu_usage, cpu_cores) = self.cpu.sample();

        let hotspot = groups
            .iter()
            .filter(|g| matches!(g.id, "cpu_p" | "cpu_e" | "gpu" | "soc"))
            .map(|g| g.max)
            .fold(0.0, f64::max);

        self.history.push_back((now, hotspot));
        while self.history.front().map_or(false, |(t, _)| now.duration_since(*t) > Duration::from_secs(30)) {
            self.history.pop_front();
        }
        let smoothed = match self.smoothed {
            Some(s) => s + 0.3 * (hotspot - s),
            None => hotspot,
        };
        self.smoothed = Some(smoothed);
        let trend = self.trend();

        // --- Sécurité : au-delà de 100 °C en mode non-Auto, on force le maximum
        if mode != Mode::Auto {
            if hotspot >= SAFETY_ON {
                self.safety = true;
            } else if hotspot < SAFETY_OFF {
                self.safety = false;
            }
        } else {
            self.safety = false;
        }

        if shared.privileged && (mode != Mode::Auto || self.applied_manual) {
            self.apply(&mode, smoothed, &fan_list);
            fan_list = fans::read_with(&self.smc, &self.fan_limits);
        }

        let advice = control::advise(smoothed, trend, &fan_list, &mode);

        // ---------- Batterie ----------
        self.ticks += 1;
        if shared.woke.swap(false, Ordering::SeqCst) {
            self.limiter.invalidate();
        }
        let mut bat = battery::read();
        if let (Some(b), Some(g)) = (bat.as_mut(), groups.iter().find(|g| g.id == "battery")) {
            if b.temperature_c <= 0.0 {
                b.temperature_c = g.max; // capteurs SMC TB0T..TB2T
            }
        }
        let (charge_cfg, pmset_backup) = {
            let s = shared.settings.lock().unwrap();
            (s.charge.clone(), s.pmset_backup.clone())
        };
        let charge = self.limiter.tick(&self.smc, &charge_cfg, bat.as_ref(), shared.privileged);
        if let Some(b) = bat.as_mut() {
            // Moyenne exponentielle (~20 s) indépendante de la cadence du tick
            let alpha = 1.0 - (-dt.min(60.0) / 20.0).exp();
            let ema = match self.power_ema {
                Some(e) => e + alpha * (b.power_w - e),
                None => b.power_w,
            };
            self.power_ema = Some(ema);
            if b.minutes_remaining.is_none() {
                if !b.external_connected && ema < -0.5 {
                    b.minutes_remaining = Some((b.current_mah * b.voltage_v / 1000.0 / -ema * 60.0).round());
                } else if b.is_charging && b.amperage_a > 0.05 {
                    let target = if charge_cfg.enabled { charge_cfg.limit as f64 } else { 100.0 };
                    let missing = (target / 100.0 * b.max_mah - b.current_mah).max(0.0);
                    b.minutes_remaining = Some((missing / 1000.0 / b.amperage_a * 60.0).round());
                }
            }
        }
        let gpu_usage = sensors::gpu_usage();
        self.usage.record(bat.as_ref(), cpu_usage, gpu_usage);
        let discharging = bat.as_ref().map_or(false, |b| b.power_w < -0.3);
        let clients = self.always_full || shared.has_clients();
        let due = |at: Option<Instant>, every: u64| at.map_or(true, |t| t.elapsed() >= Duration::from_secs(every));
        // Énergie par processus (parcourt ~800 processus en root) : toutes les 3 s si une fenêtre l'affiche,
        // sinon toutes les 15 s et seulement en décharge (statistiques par app)
        let ui = self.always_full || shared.ui_active();
        if (ui || discharging) && due(self.energy_at, if ui { 3 } else { 15 }) {
            self.energy_at = Some(now);
            self.energy.sample();
            if discharging {
                self.usage.add_apps(&self.energy.deltas, self.energy.last_dt);
            }
        }
        // pmset (lance un processus) : toutes les 2 min si l'app est ouverte, ou après un changement
        if shared.pmset_dirty.swap(false, Ordering::SeqCst) || (clients && due(self.pmset_at, 120)) {
            self.pmset_at = Some(now);
            self.pmset = power::read();
        }
        if clients && due(self.conflicts_at, 300) {
            self.conflicts_at = Some(now);
            self.conflicts = detect_conflicts();
        }
        // Rapport d'utilisation : uniquement quand la vue est affichée
        let wanted = shared.usage_wanted();
        self.usage.set_wanted(wanted);
        if wanted && self.usage.log_ready() && (self.usage.take_fresh_log() || due(self.usage_at, 30) || shared.usage.lock().unwrap().as_str() == "null") {
            self.usage_at = Some(now);
            self.publish_usage(shared, bat.as_ref());
        }
        let battery_tips = bat
            .as_ref()
            .filter(|_| clients)
            .map(|b| {
                battery_advice::tips(&battery_advice::Ctx {
                    bat: b,
                    charge: &charge,
                    pmset: self.pmset.as_ref(),
                    top: &self.energy.top,
                    fan_mode: &mode,
                    conflicts: &self.conflicts,
                    gpu_usage,
                    privileged: shared.privileged,
                })
            })
            .unwrap_or_default();

        Snapshot {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
            privileged: shared.privileged,
            cpu_model: self.cpu_model.clone(),
            hw_model: self.hw_model.clone(),
            mode,
            fans: fan_list,
            groups,
            sensors,
            cpu_usage,
            cpu_cores,
            gpu_usage,
            power_w: if !watched { None } else { self.smc.read_f64("PSTR").ok().filter(|v| v.is_finite() && *v >= 0.0).map(sensors::round1) },
            hotspot: sensors::round1(hotspot),
            trend_c_per_min: sensors::round1(trend),
            safety_override: self.safety,
            advice,
            last_error: self.last_error.clone(),
            battery: bat,
            charge,
            power_settings: self.pmset.clone(),
            pmset_backup,
            top_processes: self.energy.top.clone(),
            battery_tips,
            conflicts: self.conflicts.clone(),
        }
    }

    /// (SMC partagé, capacités de charge) pour le crochet de mise en veille.
    pub fn sleep_handles(&self) -> (Arc<Smc>, crate::charge::Caps) {
        (self.smc.clone(), self.limiter.caps)
    }

    fn apply(&mut self, mode: &Mode, temp: f64, fan_list: &[Fan]) {
        if *mode == Mode::Auto {
            if self.applied_manual {
                match fans::restore_auto(&self.smc) {
                    Ok(()) => self.last_error = None,
                    Err(e) => self.last_error = Some(format!("Retour en Auto : {e}")),
                }
                self.applied_manual = false;
                self.commanded.clear();
            }
            return;
        }
        if self.commanded.len() != fan_list.len() {
            self.commanded = fan_list.iter().map(|f| f.rpm).collect();
        }
        let mut err = None;
        for (i, fan) in fan_list.iter().enumerate() {
            let desired = if self.safety {
                fan.max
            } else {
                match mode {
                    Mode::Manual { percent } => control::rpm_for_percent(fan, *percent),
                    Mode::Curve { preset } => {
                        let want = control::rpm_for_percent(fan, preset.percent_for(temp));
                        // Rampe : monte vite, descend doucement (évite le « pompage »)
                        let prev = self.commanded[i];
                        want.clamp(prev - 150.0, prev + 600.0)
                    }
                    Mode::Auto => unreachable!(),
                }
            };
            let desired = desired.clamp(fan.min, fan.max);
            self.commanded[i] = desired;
            // Ré-applique aussi régulièrement si macOS a repris la main
            if !fan.manual || (desired - fan.target).abs() > 15.0 {
                if let Err(e) = fans::set_rpm(&self.smc, fan.id, desired) {
                    err = Some(format!("{} : {e}", fan.name));
                }
            }
        }
        self.applied_manual = true;
        self.last_error = err;
    }

    pub fn publish_usage(&self, shared: &Shared, bat: Option<&BatteryInfo>) {
        let r = self.usage.report(bat, self.pmset.as_ref(), &self.energy.top);
        if let Ok(s) = serde_json::to_string(&r) {
            *shared.usage.lock().unwrap() = s;
        }
    }

    pub fn shutdown(&mut self, privileged: bool) {
        self.usage.save();
        if privileged {
            self.limiter.shutdown(&self.smc);
        }
        if self.applied_manual {
            let _ = fans::restore_auto(&self.smc);
            self.applied_manual = false;
        }
    }

    pub fn run(mut self, shared: Arc<Shared>, exit_with_parent: bool) {
        self.usage.start_background();
        loop {
            if STOP.load(Ordering::SeqCst) || (exit_with_parent && unsafe { libc::getppid() } == 1) {
                self.shutdown(shared.privileged);
                std::process::exit(0);
            }
            let t0 = Instant::now();
            let snap = self.tick(&shared);
            if std::env::var_os("FANENGINE_DEBUG").is_some() {
                eprintln!("tick {:>5.1} ms  clients={} ui={} next={:?}", t0.elapsed().as_secs_f64() * 1e3, shared.clients.load(Ordering::SeqCst), shared.ui_active(), self.interval(&shared));
            }
            // Sérialisé seulement si quelqu'un peut le lire
            if shared.has_clients() {
                if let Ok(s) = serde_json::to_string(&snap) {
                    *shared.snapshot.lock().unwrap() = s;
                }
            }
            shared.sleep(self.interval(&shared));
        }
    }

    /// Cadence adaptative : 1 s si une fenêtre est visible ou si ThermoPilot pilote les ventilateurs,
    /// 5 s si l'app tourne en arrière-plan (barre des menus), 10 s si elle est fermée.
    fn interval(&self, shared: &Shared) -> Duration {
        let mode_auto = *shared.mode.lock().unwrap() == Mode::Auto;
        if !mode_auto || self.applied_manual || shared.ui_active() {
            Duration::from_secs(1)
        } else if shared.has_clients() {
            Duration::from_secs(5)
        } else {
            Duration::from_secs(10)
        }
    }

    pub fn smc(&self) -> &Smc {
        &self.smc
    }
}

/// Détecte d'autres outils qui pilotent la charge (conflits d'écriture SMC), sans lancer `pgrep`.
fn detect_conflicts() -> Vec<String> {
    let (mut battery_script, mut aldente) = (false, false);
    for pid in crate::energy::all_pids() {
        let Some(name) = crate::energy::pid_name(pid) else { continue };
        let lower = name.to_ascii_lowercase();
        if lower.contains("aldente") {
            aldente = true;
        } else if !battery_script && matches!(lower.as_str(), "bash" | "sh" | "zsh" | "battery") {
            if let Some(args) = crate::energy::pid_args(pid) {
                battery_script = args.contains("battery") && args.contains("maintain");
            }
        }
    }
    let mut v = vec![];
    if battery_script {
        v.push("Le script « battery » (maintain)".to_string());
    }
    if aldente {
        v.push("AlDente".to_string());
    }
    v
}
