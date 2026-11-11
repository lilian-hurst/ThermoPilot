//! Suivi de l'utilisation de la batterie :
//!  - enregistreur interne (1 point/min, énergie par app pendant la décharge, conso au repos), 7 jours ;
//!  - journal macOS `pmset -g log` (veille, réveils, niveau de charge) pour l'historique et la vidange en veille ;
//!  - analyse « est-ce normal ? » + recommandations.

use crate::battery::BatteryInfo;
use crate::battery_advice::{tip, Tip};
use crate::energy::ProcEnergy;
use crate::power::PmsetProfiles;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const KEEP_SECS: f64 = 7.0 * 86400.0;
const SAMPLE_EVERY: f64 = 60.0;
const SAVE_EVERY: Duration = Duration::from_secs(300);
const LOG_EVERY: Duration = Duration::from_secs(15 * 60);

// Repères (MacBook Pro Apple Silicon)
const AWAKE_GOOD: f64 = 8.0; // %/h — usage léger (web, bureautique)
const AWAKE_HIGH: f64 = 15.0;
const AWAKE_BAD: f64 = 25.0;
const IDLE_GOOD_W: f64 = 7.0; // W au repos écran allumé
const IDLE_BAD_W: f64 = 12.0;
const SLEEP_GOOD: f64 = 0.5; // %/h en veille (≈ 4 %/nuit)
const SLEEP_BAD: f64 = 1.5;

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// ------------------------------------------------------------------ données persistées

#[derive(Serialize, Deserialize, Clone)]
pub struct Sample {
    pub t: f64,
    pub pct: f64,
    /// Puissance batterie (W) : < 0 décharge, > 0 charge
    pub w: f64,
    pub ac: bool,
    pub cpu: f64,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct HourBucket {
    /// Temps éveillé en décharge (s) et énergie fournie par la batterie (J)
    batt_secs: f64,
    batt_j: f64,
    /// Idem, au repos (CPU < 8 %, GPU < 10 %)
    idle_secs: f64,
    idle_j: f64,
    cpu_sum: f64,
    /// Énergie par app pendant la décharge : nom -> [joules, secondes CPU]
    apps: HashMap<String, [f64; 2]>,
}

#[derive(Serialize, Deserialize, Default)]
struct UsageData {
    samples: Vec<Sample>,
    /// clé = heure unix (t / 3600)
    hours: BTreeMap<i64, HourBucket>,
}

// ------------------------------------------------------------------ journal pmset

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Sleep,
    Wake,
    DarkWake,
    Other,
}

#[derive(Clone, Debug)]
struct PmEvent {
    t: f64,
    kind: Kind,
    pct: f64,
    ac: bool,
    reason: String,
}

#[derive(Clone, Serialize)]
pub struct Blocker {
    pub pid: i32,
    pub name: String,
    pub kind: String,
    pub since: String,
    pub label: String,
}

#[derive(Default)]
struct PmCache {
    events: Vec<PmEvent>,
    blockers: Vec<Blocker>,
    last_log: Option<Instant>,
    last_assertions: Option<Instant>,
    /// Journal relu depuis le dernier rapport
    fresh: bool,
}

// ------------------------------------------------------------------ rapport

#[derive(Serialize, Clone)]
pub struct Check {
    pub id: &'static str,
    pub label: String,
    pub value: String,
    pub reference: String,
    pub detail: String,
    /// good | info | warn | bad | unknown
    pub level: &'static str,
}

#[derive(Serialize, Clone, Default)]
pub struct DaySummary {
    pub date: String,
    pub label: String,
    pub awake_batt_h: f64,
    pub awake_batt_pct: f64,
    pub sleep_batt_h: f64,
    pub sleep_batt_pct: f64,
    pub ac_h: f64,
    pub charged_pct: f64,
    pub darkwakes: usize,
}

#[derive(Serialize, Clone)]
pub struct Session {
    pub start: f64,
    pub end: f64,
    pub pct_start: f64,
    pub pct_end: f64,
    pub awake_h: f64,
    pub sleep_h: f64,
    pub awake_pct_per_h: Option<f64>,
    pub ongoing: bool,
    #[serde(skip)]
    awake_drop: f64,
}

#[derive(Serialize, Clone)]
pub struct SleepSession {
    pub start: f64,
    pub end: f64,
    pub pct_start: f64,
    pub pct_end: f64,
    pub hours: f64,
    pub pct_per_h: f64,
    pub darkwakes: usize,
    pub on_battery: bool,
    pub main_reason: Option<String>,
    pub level: &'static str,
}

#[derive(Serialize, Clone)]
pub struct AppUsage {
    pub name: String,
    pub wh: f64,
    pub share_pct: f64,
    pub cpu_min: f64,
    pub avg_w: f64,
}

#[derive(Serialize, Clone)]
pub struct Reason {
    pub label: String,
    pub count: usize,
}

#[derive(Serialize, Clone)]
pub struct TimelinePoint {
    pub t: f64,
    pub pct: f64,
    pub ac: bool,
    pub asleep: bool,
}

#[derive(Serialize, Clone)]
pub struct UsageReport {
    pub generated_at: f64,
    pub recording_since: Option<f64>,
    pub log_since: Option<f64>,
    pub verdict: Check,
    pub checks: Vec<Check>,
    pub days: Vec<DaySummary>,
    pub sessions: Vec<Session>,
    pub sleeps: Vec<SleepSession>,
    pub wake_reasons: Vec<Reason>,
    pub apps_24h: Vec<AppUsage>,
    pub apps_7d: Vec<AppUsage>,
    pub total_24h_wh: f64,
    pub total_7d_wh: f64,
    pub other_24h_wh: f64,
    pub other_7d_wh: f64,
    pub battery_hours_24h: f64,
    pub battery_hours_7d: f64,
    pub blockers: Vec<Blocker>,
    pub timeline: Vec<TimelinePoint>,
    pub tips: Vec<Tip>,
}

// ------------------------------------------------------------------ enregistreur

pub struct Recorder {
    path: PathBuf,
    data: UsageData,
    last_tick: Option<Instant>,
    last_sample: f64,
    last_save: Instant,
    cache: Arc<Mutex<PmCache>>,
    wanted: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::Thread>,
}

impl Recorder {
    pub fn new() -> Recorder {
        let path = if unsafe { libc::geteuid() } == 0 {
            PathBuf::from("/Library/Application Support/ThermoPilot/usage.json")
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join("Library/Application Support/ThermoPilot/usage-user.json")
        };
        let data = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Recorder {
            path,
            data,
            last_tick: None,
            last_sample: 0.0,
            last_save: Instant::now(),
            cache: Arc::new(Mutex::new(PmCache::default())),
            wanted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            worker: None,
        }
    }

    /// Thread d'arrière-plan, endormi tant que la vue « Utilisation » n'est pas affichée :
    /// journal pmset (lourd, ~2 s CPU) au plus toutes les 15 min, assertions au plus chaque minute.
    pub fn start_background(&mut self) {
        let cache = self.cache.clone();
        let wanted = self.wanted.clone();
        let h = std::thread::spawn(move || loop {
            if wanted.load(std::sync::atomic::Ordering::Relaxed) {
                refresh(&cache, false);
            }
            std::thread::park_timeout(Duration::from_secs(60));
        });
        self.worker = Some(h.thread().clone());
    }

    /// Indique si l'app affiche le rapport (réveille le thread de lecture au besoin).
    pub fn set_wanted(&self, on: bool) {
        let was = self.wanted.swap(on, std::sync::atomic::Ordering::Relaxed);
        if on && !was {
            if let Some(w) = &self.worker {
                w.unpark();
            }
        }
    }

    /// Journal macOS lu au moins une fois (sinon le rapport serait incomplet).
    pub fn log_ready(&self) -> bool {
        self.cache.lock().unwrap().last_log.is_some()
    }

    /// Vrai une fois après chaque relecture du journal.
    pub fn take_fresh_log(&self) -> bool {
        std::mem::take(&mut self.cache.lock().unwrap().fresh)
    }

    /// Lecture immédiate (commande `fanengine usage`).
    pub fn refresh_blocking(&self) {
        refresh(&self.cache, true);
    }

    /// Appelé à chaque tick (1 s).
    pub fn record(&mut self, bat: Option<&BatteryInfo>, cpu: Option<f64>, gpu: Option<f64>) {
        let now_i = Instant::now();
        let dt = self.last_tick.map(|l| now_i.duration_since(l).as_secs_f64()).unwrap_or(0.0);
        self.last_tick = Some(now_i);
        let Some(b) = bat else { return };
        let t = now();
        let cpu = cpu.unwrap_or(0.0);
        // Décharge réelle (inclut le mode « adaptateur coupé » du limiteur)
        // dt < 30 s : la cadence du moteur va de 1 à 10 s ; au-delà, c'était une veille
        if dt > 0.0 && dt < 30.0 && b.power_w < -0.3 {
            let h = self.data.hours.entry((t / 3600.0).floor() as i64).or_default();
            let j = -b.power_w * dt;
            h.batt_secs += dt;
            h.batt_j += j;
            h.cpu_sum += cpu * dt;
            if cpu < 8.0 && gpu.unwrap_or(0.0) < 10.0 {
                h.idle_secs += dt;
                h.idle_j += j;
            }
        }
        if t - self.last_sample >= SAMPLE_EVERY {
            self.last_sample = t;
            self.data.samples.push(Sample {
                t,
                pct: b.percent,
                w: (b.power_w * 10.0).round() / 10.0,
                ac: b.external_connected,
                cpu: cpu.round(),
            });
            let cutoff = t - KEEP_SECS;
            if self.data.samples.first().map_or(false, |s| s.t < cutoff) {
                self.data.samples.retain(|s| s.t >= cutoff);
            }
            let hcut = (cutoff / 3600.0).floor() as i64;
            self.data.hours.retain(|k, _| *k >= hcut);
        }
        if self.last_save.elapsed() >= SAVE_EVERY {
            self.save();
        }
    }

    /// Énergie par app du dernier intervalle de mesure (à n'appeler qu'en décharge).
    pub fn add_apps(&mut self, deltas: &[(String, f64, f64)], dt: f64) {
        if dt <= 0.0 || dt > 45.0 {
            return; // trou (veille) : on n'attribue rien
        }
        let h = self.data.hours.entry((now() / 3600.0).floor() as i64).or_default();
        for (name, j, cpu_s) in deltas {
            if *j <= 0.0 && *cpu_s <= 0.0 {
                continue;
            }
            let e = h.apps.entry(name.clone()).or_insert([0.0, 0.0]);
            e[0] += j;
            e[1] += cpu_s;
        }
    }

    pub fn save(&mut self) {
        self.last_save = Instant::now();
        // Allège : arrondit et retire les apps négligeables
        for h in self.data.hours.values_mut() {
            h.apps.retain(|_, v| v[0] >= 0.5 || v[1] >= 1.0);
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(&self.data) {
            let tmp = self.path.with_extension("tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    pub fn report(&self, bat: Option<&BatteryInfo>, pmset: Option<&PmsetProfiles>, top: &[ProcEnergy]) -> UsageReport {
        let (events, blockers) = {
            let c = self.cache.lock().unwrap();
            (c.events.clone(), c.blockers.clone())
        };
        build_report(&self.data, &events, blockers, bat, pmset, top)
    }
}

// ------------------------------------------------------------------ lecture pmset

fn refresh(cache: &Mutex<PmCache>, force: bool) {
    let due = force || cache.lock().unwrap().last_log.map_or(true, |l| l.elapsed() >= LOG_EVERY);
    if due {
        if let Ok(out) = Command::new("/usr/bin/pmset").args(["-g", "log"]).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            let events = parse_log(&text);
            let mut c = cache.lock().unwrap();
            c.events = events;
            c.last_log = Some(Instant::now());
            c.fresh = true;
        }
    }
    let due = force || cache.lock().unwrap().last_assertions.map_or(true, |l| l.elapsed() >= Duration::from_secs(60));
    if due {
        if let Ok(out) = Command::new("/usr/bin/pmset").args(["-g", "assertions"]).output() {
            let blockers = parse_assertions(&String::from_utf8_lossy(&out.stdout));
            let mut c = cache.lock().unwrap();
            c.blockers = blockers;
            c.last_assertions = Some(Instant::now());
        }
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "2026-11-11 08:51:13 +0900" -> temps unix
fn parse_time(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() < 25 || b[4] != b'-' || b[10] != b' ' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let sign = if b[20] == b'-' { -1 } else { 1 };
    let off = sign * (n(21..23)? * 3600 + n(23..25)? * 60);
    Some((days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - off) as f64)
}

fn parse_charge(msg: &str) -> Option<(bool, f64)> {
    let i = msg.find("Using ")?;
    let rest = &msg[i + 6..];
    let ac = rest.starts_with("AC");
    if !ac && !rest.to_ascii_lowercase().starts_with("batt") {
        return None;
    }
    let j = rest.find("Charge:")?;
    let tail = rest[j + 7..].trim_start();
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    // Lignes tronquées par macOS (« Charge: 8 » au lieu de 80) : on exige « % » ou « ) » juste après
    if !matches!(tail[digits.len()..].chars().next(), Some('%') | Some(')')) {
        return None;
    }
    Some((ac, digits.parse().ok()?))
}

fn wake_reason(msg: &str) -> String {
    let m = msg.to_ascii_lowercase();
    let r = if m.contains("rtc/maintenance") {
        "Maintenance réseau (iCloud, Localiser, mail…)"
    } else if m.contains("rtc/sleepservice") {
        "Tâches planifiées (Sleep Service)"
    } else if m.contains("bluetooth") {
        "Bluetooth (appareil connecté)"
    } else if m.contains("wifibt") {
        "Wi-Fi / réseau"
    } else if m.contains("usb") {
        "Branchement USB-C"
    } else if m.contains("hid") || m.contains("useractivity") {
        "Clavier / trackpad"
    } else if m.contains("notification") {
        "Notification"
    } else {
        "Autre"
    };
    r.to_string()
}

fn sleep_reason(msg: &str) -> String {
    if msg.contains("Clamshell") {
        "Écran rabattu".into()
    } else if msg.contains("Idle Sleep") {
        "Inactivité".into()
    } else if msg.contains("Software Sleep") {
        "Menu Pomme / logiciel".into()
    } else {
        "Rendormissement".into()
    }
}

fn parse_log(text: &str) -> Vec<PmEvent> {
    let mut v = Vec::new();
    for line in text.lines() {
        let Some(t) = parse_time(line) else { continue };
        let Some((head, msg)) = line.split_once('\t') else { continue };
        let kind = match head.get(26..).unwrap_or("").trim() {
            "Sleep" => Kind::Sleep,
            "Wake" => Kind::Wake,
            "DarkWake" => Kind::DarkWake,
            "Assertions" => Kind::Other,
            _ => continue,
        };
        let Some((ac, pct)) = parse_charge(msg) else { continue };
        let reason = match kind {
            Kind::DarkWake => wake_reason(msg),
            Kind::Sleep => sleep_reason(msg),
            _ => String::new(),
        };
        v.push(PmEvent { t, kind, pct, ac, reason });
    }
    v.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
    v
}

const SYSTEM_HOLDERS: &[&str] = &["powerd", "WindowServer", "bluetoothd", "runningboardd", "sharingd", "loginwindow", "kernel_task"];

fn parse_assertions(text: &str) -> Vec<Blocker> {
    let mut v: Vec<Blocker> = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        if line.starts_with("Listed by owning process") {
            in_list = true;
            continue;
        }
        if line.starts_with("Kernel Assertions") {
            break;
        }
        if !in_list {
            continue;
        }
        // "   pid 72596(Slack): [0x…] 00:05:04 NoIdleSleepAssertion named: \"Electron\""
        let t = line.trim();
        let Some(rest) = t.strip_prefix("pid ") else { continue };
        let Some(p) = rest.find('(') else { continue };
        let Some(q) = rest.find("):") else { continue };
        let pid: i32 = rest[..p].parse().unwrap_or(0);
        let name = rest[p + 1..q].to_string();
        let after = rest[q + 2..].trim();
        let after = after.split_once("] ").map(|x| x.1).unwrap_or(after);
        let mut it = after.split_whitespace();
        let since = it.next().unwrap_or("").to_string();
        let kind = it.next().unwrap_or("").to_string();
        let blocks = matches!(
            kind.as_str(),
            "PreventUserIdleSystemSleep" | "NoIdleSleepAssertion" | "PreventSystemSleep" | "PreventUserIdleDisplaySleep" | "NoDisplaySleepAssertion"
        );
        if !blocks || SYSTEM_HOLDERS.contains(&name.as_str()) {
            continue;
        }
        let label = after.split_once("named: ").map(|x| x.1.trim().trim_matches('"').to_string()).unwrap_or_default();
        if v.iter().any(|b| b.pid == pid && b.kind == kind) {
            continue;
        }
        v.push(Blocker { pid, name, kind, since, label });
    }
    v
}

// ------------------------------------------------------------------ analyse

fn local_day(t: f64) -> (String, String) {
    let tt = t as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&tt, &mut tm) };
    const DAYS: [&str; 7] = ["Dim.", "Lun.", "Mar.", "Mer.", "Jeu.", "Ven.", "Sam."];
    (
        format!("{:04}-{:02}-{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday),
        format!("{} {}", DAYS[(tm.tm_wday as usize) % 7], tm.tm_mday),
    )
}

#[derive(Clone, Copy)]
struct Point<'a> {
    reason: &'a str,
    t: f64,
    pct: f64,
    ac: bool,
    kind: Kind,
    sample: bool,
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn fmt_h(h: f64) -> String {
    let m = (h * 60.0).round() as i64;
    if m >= 60 { format!("{} h {:02}", m / 60, m % 60) } else { format!("{m} min") }
}

fn apps_since(data: &UsageData, from_hour: i64) -> (Vec<AppUsage>, f64, f64, f64) {
    let mut apps: HashMap<&str, [f64; 2]> = HashMap::new();
    let (mut total_j, mut secs) = (0.0, 0.0);
    for (_, h) in data.hours.range(from_hour..) {
        total_j += h.batt_j;
        secs += h.batt_secs;
        for (n, v) in &h.apps {
            let e = apps.entry(n.as_str()).or_insert([0.0, 0.0]);
            e[0] += v[0];
            e[1] += v[1];
        }
    }
    let app_j: f64 = apps.values().map(|v| v[0]).sum();
    // L'estimation par processus peut dépasser la mesure batterie : on prend la plus grande des deux
    let denom = total_j.max(app_j).max(1.0);
    let mut v: Vec<AppUsage> = apps
        .into_iter()
        .map(|(n, e)| AppUsage {
            name: n.to_string(),
            wh: r2(e[0] / 3600.0),
            share_pct: r1(e[0] / denom * 100.0),
            cpu_min: r1(e[1] / 60.0),
            avg_w: if secs > 0.0 { r2(e[0] / secs) } else { 0.0 },
        })
        .filter(|a| a.wh >= 0.01 || a.cpu_min >= 0.5)
        .collect();
    v.sort_by(|a, b| b.wh.partial_cmp(&a.wh).unwrap());
    v.truncate(12);
    let other = r2(((total_j - app_j).max(0.0)) / 3600.0);
    (v, r2(total_j / 3600.0), other, secs / 3600.0)
}

/// Conseil spécifique selon l'app.
fn app_hint(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("chrome") || n.contains("brave") || n.contains("edge") || n.contains("opera") {
        "Les navigateurs Chromium consomment plus que Safari : fermez les onglets inutiles (vidéo, pages lourdes), suspendez les onglets inactifs, ou utilisez Safari sur batterie."
    } else if n.contains("firefox") {
        "Fermez les onglets lourds (vidéo, pages qui se rafraîchissent) ; Safari est plus économe sur batterie."
    } else if n.contains("safari") || n.contains("webkit") {
        "Un onglet est probablement actif en arrière-plan (vidéo, animation, page qui se rafraîchit) : Fenêtre › Activité du Safari pour l'identifier."
    } else if n == "windowserver" {
        "C'est le rendu de l'écran : baissez la luminosité, évitez les fonds animés, débranchez un écran externe inutile ou limitez le rafraîchissement à 60 Hz sur batterie."
    } else if n == "kernel_task" {
        "kernel_task grimpe quand le Mac est chaud : laissez-le refroidir, surélevez-le, évitez de l'utiliser sur un lit ou un coussin."
    } else if n.starts_with("mds") || n.starts_with("mdworker") || n == "corespotlightd" {
        "Indexation Spotlight (temporaire après une mise à jour ou beaucoup de nouveaux fichiers). Excluez les gros dossiers (node_modules, datasets, VM) dans Réglages › Spotlight."
    } else if n == "photoanalysisd" || n == "mediaanalysisd" || n == "photolibraryd" {
        "Analyse de la photothèque : elle se fait normalement sur secteur. Laissez le Mac branché une nuit pour qu'elle se termine."
    } else if n == "backupd" || n.contains("timemachine") {
        "Sauvegarde Time Machine en cours : planifiez-la sur secteur si possible."
    } else if n.contains("docker") || n.contains("virtualization") || n.contains("qemu") || n.contains("utm") || n.contains("parallels") || n.contains("tart") {
        "Une machine virtuelle / Docker tourne en continu : arrêtez les conteneurs ou VM inutiles sur batterie."
    } else if n.contains("ollama") || n.contains("lm studio") || n.contains("lmstudio") || n.starts_with("python") || n.contains("jupyter") {
        "Calcul lourd (modèle IA, script Python…) : réservez ces tâches au secteur, ou arrêtez les serveurs de modèles qui restent chargés."
    } else if n.contains("helper") || n.contains("electron") || ["claude", "slack", "discord", "code", "cursor", "notion", "spotify", "teams", "whatsapp"].iter().any(|k| n.contains(k)) {
        "App basée sur un navigateur intégré (Electron) : elle consomme même en arrière-plan. Quittez-la quand vous ne l'utilisez pas, ou utilisez la version web dans Safari."
    } else if n.contains("zoom") || n.contains("facetime") || n.contains("webex") {
        "La visio sollicite caméra, encodage vidéo et réseau : coupez la vidéo quand elle n'est pas nécessaire."
    } else {
        "Quittez-la quand vous ne l'utilisez pas, ou vérifiez ses réglages (synchronisation, animations, actualisation en arrière-plan)."
    }
}

fn level_rank(l: &str) -> u8 {
    match l {
        "bad" => 4,
        "warn" => 3,
        "info" => 2,
        "good" => 1,
        _ => 0,
    }
}

fn build_report(
    data: &UsageData,
    events: &[PmEvent],
    blockers: Vec<Blocker>,
    bat: Option<&BatteryInfo>,
    pmset: Option<&PmsetProfiles>,
    top: &[ProcEnergy],
) -> UsageReport {
    let t_now = now();
    let since = t_now - KEEP_SECS;

    // ---- points fusionnés : journal macOS + échantillons internes
    let mut pts: Vec<Point> = events
        .iter()
        .filter(|e| e.t >= since - 86400.0)
        .map(|e| Point { reason: &e.reason, t: e.t, pct: e.pct, ac: e.ac, kind: e.kind, sample: false })
        .collect();
    for s in &data.samples {
        // En mode « adaptateur coupé » le Mac est branché mais se décharge : on le compte comme sur batterie
        let on_batt = s.w < -0.3;
        pts.push(Point { reason: "", t: s.t, pct: s.pct.round(), ac: s.ac && !on_batt, kind: Kind::Other, sample: true });
    }
    if let Some(b) = bat {
        pts.push(Point { reason: "", t: t_now, pct: b.percent, ac: b.external_connected && b.power_w >= -0.3, kind: Kind::Other, sample: true });
    }
    pts.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

    // ---- parcours chronologique
    let mut days: BTreeMap<String, DaySummary> = BTreeMap::new();
    let mut asleep = false;
    let mut sessions: Vec<Session> = vec![];
    let mut cur: Option<Session> = None;
    let mut sleeps: Vec<SleepSession> = vec![];
    let mut cur_sleep: Option<(f64, f64, usize, bool, HashMap<String, usize>)> = None;
    let mut reasons: HashMap<String, usize> = HashMap::new();
    let mut timeline: Vec<TimelinePoint> = vec![];
    let mut last_tl = 0.0;
    let (mut awake_h, mut awake_pct, mut sleep_h, mut sleep_pct) = (0.0, 0.0, 0.0, 0.0);
    let (mut awake_h24, mut awake_pct24) = (0.0, 0.0);

    for i in 0..pts.len() {
        let p = pts[i];
        // Changement d'état
        match p.kind {
            Kind::Sleep => {
                if !asleep {
                    cur_sleep = Some((p.t, p.pct, 0, !p.ac, HashMap::new()));
                }
                asleep = true;
            }
            Kind::DarkWake => {
                if let Some(s) = cur_sleep.as_mut() {
                    s.2 += 1;
                    if p.ac {
                        s.3 = false;
                    }
                    *s.4.entry(p.reason.to_string()).or_default() += 1;
                }
                if p.t >= since {
                    *reasons.entry(p.reason.to_string()).or_default() += 1;
                    let (d, l) = local_day(p.t);
                    days.entry(d.clone()).or_insert_with(|| DaySummary { date: d, label: l, ..Default::default() }).darkwakes += 1;
                }
            }
            Kind::Wake => {
                if let Some((st, sp, dw, batt, rs)) = cur_sleep.take() {
                    let hours = (p.t - st) / 3600.0;
                    if p.t >= since && hours >= 0.25 {
                        let drop = (sp - p.pct).max(0.0);
                        let on_battery = batt && !p.ac;
                        let rate = if hours > 0.0 { drop / hours } else { 0.0 };
                        let level = if !on_battery {
                            "unknown"
                        } else if hours < 2.0 {
                            "info" // trop court pour juger (niveau en % entiers)
                        } else if rate <= SLEEP_GOOD {
                            "good"
                        } else if rate <= SLEEP_BAD {
                            "warn"
                        } else {
                            "bad"
                        };
                        sleeps.push(SleepSession {
                            start: st,
                            end: p.t,
                            pct_start: sp,
                            pct_end: p.pct,
                            hours: r2(hours),
                            pct_per_h: r2(rate),
                            darkwakes: dw,
                            on_battery,
                            main_reason: rs.into_iter().max_by_key(|x| x.1).map(|x| x.0),
                            level,
                        });
                    }
                }
                asleep = false;
            }
            Kind::Other => {}
        }

        // Courbe (≤ 1 point / 5 min, + tous les événements)
        if p.t >= since && (!p.sample || p.t - last_tl >= 300.0 || i == pts.len() - 1) {
            timeline.push(TimelinePoint { t: p.t, pct: p.pct, ac: p.ac, asleep });
            last_tl = p.t;
        }

        // Sessions de décharge
        if !p.ac {
            match cur.as_mut() {
                Some(s) => {
                    s.end = p.t;
                    s.pct_end = p.pct;
                }
                None => {
                    cur = Some(Session { start: p.t, end: p.t, pct_start: p.pct, pct_end: p.pct, awake_h: 0.0, sleep_h: 0.0, awake_pct_per_h: None, ongoing: false, awake_drop: 0.0 })
                }
            }
        } else if let Some(s) = cur.take() {
            if s.end - s.start >= 600.0 && s.end >= since {
                sessions.push(s);
            }
        }

        // Intervalle jusqu'au point suivant
        let Some(q) = pts.get(i + 1) else { continue };
        let dt_h = (q.t - p.t) / 3600.0;
        if dt_h <= 0.0 || p.t < since {
            continue;
        }
        if !asleep && dt_h > 3.0 {
            continue; // trou (Mac éteint, journal manquant)
        }
        let (d, l) = local_day(p.t);
        let day = days.entry(d.clone()).or_insert_with(|| DaySummary { date: d, label: l, ..Default::default() });
        let delta = p.pct - q.pct;
        if !p.ac && !q.ac {
            if asleep {
                day.sleep_batt_h += dt_h;
                day.sleep_batt_pct += delta;
                sleep_h += dt_h;
                sleep_pct += delta;
            } else {
                day.awake_batt_h += dt_h;
                day.awake_batt_pct += delta;
                awake_h += dt_h;
                awake_pct += delta;
                if p.t >= t_now - 86400.0 {
                    awake_h24 += dt_h;
                    awake_pct24 += delta;
                }
            }
            if let Some(s) = cur.as_mut() {
                if asleep {
                    s.sleep_h += dt_h
                } else {
                    s.awake_h += dt_h;
                    s.awake_drop += delta;
                }
            }
        } else if p.ac && q.ac {
            day.ac_h += dt_h;
            day.charged_pct += (q.pct - p.pct).max(0.0);
        }
    }
    if let Some(mut s) = cur.take() {
        s.ongoing = true;
        if s.end - s.start >= 300.0 {
            sessions.push(s);
        }
    }
    for s in &mut sessions {
        if s.awake_h >= 0.25 {
            s.awake_pct_per_h = Some(r1(s.awake_drop.max(0.0) / s.awake_h));
        }
        s.awake_h = r2(s.awake_h);
        s.sleep_h = r2(s.sleep_h);
    }
    sessions.reverse();
    sessions.truncate(15);
    sleeps.reverse();
    sleeps.truncate(15);
    let mut days: Vec<DaySummary> = days
        .into_values()
        .map(|mut d| {
            d.awake_batt_h = r2(d.awake_batt_h);
            d.awake_batt_pct = r1(d.awake_batt_pct.max(0.0));
            d.sleep_batt_h = r2(d.sleep_batt_h);
            d.sleep_batt_pct = r1(d.sleep_batt_pct.max(0.0));
            d.ac_h = r2(d.ac_h);
            d.charged_pct = r1(d.charged_pct);
            d
        })
        .collect();
    days.reverse();
    days.truncate(8);
    let mut wake_reasons: Vec<Reason> = reasons.into_iter().map(|(label, count)| Reason { label, count }).collect();
    wake_reasons.sort_by(|a, b| b.count.cmp(&a.count));

    // ---- énergie par app
    let h_now = (t_now / 3600.0).floor() as i64;
    let (apps_24h, total_24h_wh, other_24h_wh, bh24) = apps_since(data, h_now - 23);
    let (apps_7d, total_7d_wh, other_7d_wh, bh7) = apps_since(data, h_now - 24 * 7);
    let (mut idle_s, mut idle_j, mut b_s, mut b_j, mut cpu_sum) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (_, h) in data.hours.range(h_now - 24 * 7..) {
        idle_s += h.idle_secs;
        idle_j += h.idle_j;
        b_s += h.batt_secs;
        b_j += h.batt_j;
        cpu_sum += h.cpu_sum;
    }

    // ---- vérifications
    let mut checks = vec![];
    let cap_wh = bat.map(|b| b.max_mah * b.voltage_v.max(10.0) / 1000.0).unwrap_or(0.0);

    // 1. Éveillé sur batterie
    let (rate_h, rate_p, window) = if awake_h24 >= 0.5 { (awake_h24, awake_pct24, "24 h") } else { (awake_h, awake_pct, "7 jours") };
    let avg_cpu = if b_s > 0.0 { cpu_sum / b_s } else { 0.0 };
    let avg_w = if b_s > 60.0 { Some(b_j / b_s) } else { None };
    if rate_h >= 0.5 {
        let r = (rate_p / rate_h).max(0.0);
        let heavy = avg_cpu >= 35.0;
        let level = if r <= AWAKE_GOOD {
            "good"
        } else if r <= AWAKE_HIGH {
            "info"
        } else if r <= AWAKE_BAD {
            if heavy { "info" } else { "warn" }
        } else if heavy {
            "warn"
        } else {
            "bad"
        };
        let autonomy = if r > 0.1 { format!(" — soit ≈ {} pour une charge complète", fmt_h(100.0 / r)) } else { String::new() };
        let mut detail = format!("Moyenne sur {window} d'utilisation sur batterie ({}){autonomy}.", fmt_h(rate_h));
        if let Some(w) = avg_w {
            detail += &format!(" Puissance moyenne mesurée : {:.1} W, CPU moyen {:.0} %.", w, avg_cpu);
        }
        detail += match level {
            "good" => " Normal pour un usage léger à modéré.",
            "info" if heavy => " Élevé mais cohérent avec la charge de travail (CPU très sollicité).",
            "info" => " Normal pour un usage soutenu (développement, nombreuses apps ouvertes).",
            "warn" if heavy => " Très élevé, explicable par la charge de travail : réservez les tâches lourdes au secteur.",
            "warn" => " Plus élevé que la normale pour la charge observée : voyez les apps ci-dessous.",
            _ => " Anormalement élevé : une app ou un processus consomme fortement en continu.",
        };
        checks.push(Check {
            id: "awake",
            label: "Décharge en utilisation".into(),
            value: format!("{:.1} %/h", r),
            reference: format!("normal ≤ {AWAKE_GOOD:.0} %/h (léger), ≤ {AWAKE_HIGH:.0} %/h (soutenu)"),
            detail,
            level,
        });
    } else {
        checks.push(Check {
            id: "awake",
            label: "Décharge en utilisation".into(),
            value: "—".into(),
            reference: format!("normal ≤ {AWAKE_GOOD:.0} %/h"),
            detail: "Pas encore assez de temps d'utilisation sur batterie (30 min minimum).".into(),
            level: "unknown",
        });
    }

    // 2. Au repos
    if idle_s >= 600.0 {
        let w = idle_j / idle_s;
        let level = if w <= IDLE_GOOD_W { "good" } else if w <= IDLE_BAD_W { "warn" } else { "bad" };
        let pct_h = if cap_wh > 0.0 { format!(" (≈ {:.1} %/h)", w / cap_wh * 100.0) } else { String::new() };
        checks.push(Check {
            id: "idle",
            label: "Consommation au repos (écran allumé)".into(),
            value: format!("{:.1} W", w),
            reference: format!("normal ≤ {IDLE_GOOD_W:.0} W"),
            detail: format!(
                "Mesurée sur {} où le CPU et le GPU étaient presque inactifs{pct_h}. {}",
                fmt_h(idle_s / 3600.0),
                match level {
                    "good" => "Normal : l'écran est le principal consommateur.",
                    "warn" => "Un peu élevé : luminosité forte, écran externe, ou processus en arrière-plan.",
                    _ => "Anormal : quelque chose consomme en permanence en arrière-plan (voir les apps).",
                }
            ),
            level,
        });
    } else {
        checks.push(Check {
            id: "idle",
            label: "Consommation au repos (écran allumé)".into(),
            value: "—".into(),
            reference: format!("normal ≤ {IDLE_GOOD_W:.0} W"),
            detail: "Mesure en cours : il faut ~10 min au repos sur batterie (enregistrée par ThermoPilot).".into(),
            level: "unknown",
        });
    }

    // 3. En veille
    if sleep_h >= 2.0 {
        let r = (sleep_pct / sleep_h).max(0.0);
        let level = if r <= SLEEP_GOOD { "good" } else if r <= SLEEP_BAD { "warn" } else { "bad" };
        let dw: usize = sleeps.iter().filter(|s| s.on_battery).map(|s| s.darkwakes).sum();
        let sh: f64 = sleeps.iter().filter(|s| s.on_battery).map(|s| s.hours).sum();
        let dw_txt = if sh > 0.0 { format!(" {:.1} réveils en arrière-plan par heure.", dw as f64 / sh) } else { String::new() };
        checks.push(Check {
            id: "sleep",
            label: "Vidange en veille".into(),
            value: format!("{:.2} %/h", r),
            reference: format!("normal ≤ {SLEEP_GOOD} %/h (≈ 4 % par nuit)"),
            detail: format!(
                "{} de veille sur batterie analysées, {:.0} % perdus (≈ {:.0} % sur 8 h).{dw_txt} {}",
                fmt_h(sleep_h),
                sleep_pct.max(0.0),
                r * 8.0,
                match level {
                    "good" => "Normal.",
                    "warn" => "Un peu élevé : le Mac se réveille souvent (réseau, Bluetooth, tâches).",
                    _ => "Anormal : le Mac ne dort pas vraiment (réveils fréquents ou app qui l'en empêche).",
                }
            ),
            level,
        });
    } else {
        checks.push(Check {
            id: "sleep",
            label: "Vidange en veille".into(),
            value: "—".into(),
            reference: format!("normal ≤ {SLEEP_GOOD} %/h"),
            detail: "Pas assez de veille sur batterie dans le journal macOS (2 h minimum).".into(),
            level: "unknown",
        });
    }

    // ---- verdict global
    let worst = checks.iter().max_by_key(|c| level_rank(c.level)).cloned();
    let known = checks.iter().filter(|c| c.level != "unknown").count();
    let verdict = match worst {
        _ if known == 0 => Check {
            id: "verdict",
            label: "Analyse en cours".into(),
            value: String::new(),
            reference: String::new(),
            detail: "ThermoPilot a besoin d'un peu d'utilisation sur batterie pour juger la décharge.".into(),
            level: "unknown",
        },
        Some(c) if c.level == "bad" => Check {
            id: "verdict",
            label: "Décharge anormale".into(),
            value: String::new(),
            reference: String::new(),
            detail: format!("{} : {}. Suivez les recommandations ci-dessous.", c.label, c.value),
            level: "bad",
        },
        Some(c) if c.level == "warn" => Check {
            id: "verdict",
            label: "Décharge un peu élevée".into(),
            value: String::new(),
            reference: String::new(),
            detail: format!("{} : {} (repère : {}).", c.label, c.value, c.reference),
            level: "warn",
        },
        _ => Check {
            id: "verdict",
            label: "Décharge normale".into(),
            value: String::new(),
            reference: String::new(),
            detail: "La batterie se vide à un rythme normal pour votre utilisation.".into(),
            level: "good",
        },
    };

    // ---- recommandations
    let mut tips: Vec<Tip> = vec![];
    let running = |name: &str| top.iter().find(|p| p.name == name).map(|p| p.pid);
    let ref_apps = if total_24h_wh >= 1.0 { (&apps_24h, total_24h_wh, "24 h") } else { (&apps_7d, total_7d_wh, "7 jours") };
    for a in ref_apps.0.iter().take(4) {
        if a.share_pct < 12.0 || a.wh < 0.5 {
            continue;
        }
        let level = if a.share_pct >= 35.0 { "warn" } else { "info" };
        let mut t = tip(
            "app_energy",
            level,
            format!("{} : {:.0} % de l'énergie sur batterie", a.name, a.share_pct),
            format!("{:.1} Wh sur {} ({:.1} W en moyenne, {:.0} min CPU). {}", a.wh, ref_apps.2, a.avg_w, a.cpu_min, app_hint(&a.name)),
        );
        if let Some(pid) = running(&a.name) {
            if !SYSTEM_HOLDERS.contains(&a.name.as_str()) && a.name != "kernel_task" && pid > 1 {
                t = t.act(format!("quit:{pid}:{}", a.name), "Quitter");
            }
        }
        tips.push(t);
    }
    if ref_apps.1 >= 1.0 {
        let other = if ref_apps.2 == "24 h" { other_24h_wh } else { other_7d_wh };
        if other / ref_apps.1 >= 0.55 {
            tips.push(tip(
                "display_share",
                "info",
                format!("Écran et système : {:.0} % de l'énergie", other / ref_apps.1 * 100.0),
                "La part non attribuée aux apps correspond surtout à l'écran (rétroéclairage), au Wi-Fi et au reste de la carte mère. Baisser la luminosité de 2-3 crans est le levier le plus efficace.".to_string(),
            ));
        }
    }
    for b in &blockers {
        // "HH:MM:SS" : au-delà de 30 min, c'est probablement un oubli
        let mins = b.since.split(':').filter_map(|x| x.parse::<u64>().ok()).fold(0, |a, x| a * 60 + x) / 60;
        tips.push(
            tip(
                "blocker",
                if mins >= 30 { "warn" } else { "info" },
                format!("{} empêche la mise en veille", b.name),
                format!(
                    "Assertion « {} »{} active depuis {}. Tant qu'elle est active, le Mac ne se met pas en veille seul et l'écran peut rester allumé.",
                    b.kind,
                    if b.label.is_empty() { String::new() } else { format!(" ({})", b.label) },
                    b.since
                ),
            )
            .act(format!("quit:{}:{}", b.pid, b.name), "Quitter"),
        );
    }
    let sleep_check = checks.iter().find(|c| c.id == "sleep").map(|c| c.level).unwrap_or("unknown");
    if matches!(sleep_check, "warn" | "bad") {
        let batt = pmset.map(|p| &p.battery);
        let get = |k: &str| batt.and_then(|m| m.get(k)).copied();
        let top_reason = wake_reasons.first().map(|r| r.label.clone()).unwrap_or_default();
        if get("tcpkeepalive") == Some(1) {
            tips.push(
                tip(
                    "tcpkeepalive",
                    "warn",
                    "Couper le réseau pendant la veille",
                    format!("La cause principale des réveils est « {top_reason} ». Désactiver « TCP keep-alive » sur batterie supprime la plupart des réveils réseau (Localiser mon Mac ne pourra plus localiser le Mac endormi)."),
                )
                .act("pmset:battery:tcpkeepalive:0", "Désactiver sur batterie"),
            );
        }
        if get("powernap") == Some(1) {
            tips.push(tip("powernap_sleep", "warn", "Désactiver Power Nap sur batterie", "Power Nap réveille le Mac pour synchroniser mail, iCloud et sauvegardes.").act("pmset:battery:powernap:0", "Désactiver"));
        }
        if top_reason.starts_with("Bluetooth") {
            tips.push(tip("bt_wake", "info", "Le Bluetooth réveille le Mac", "Un appareil Bluetooth (souris, casque, manette) réveille le Mac en veille. Éteignez-le ou déconnectez-le, ou désactivez « Autoriser les appareils Bluetooth à réactiver l'ordinateur » (Réglages › Bluetooth › Avancé)."));
        }
    } else if sleep_check == "good" {
        tips.push(tip("sleep_ok", "good", "Veille saine", "Le Mac perd peu de batterie quand il dort : rien à changer."));
    }
    if checks.iter().any(|c| c.id == "idle" && c.level != "good" && c.level != "unknown") {
        tips.push(
            tip(
                "idle_high",
                "info",
                "Réduire la consommation au repos",
                "Baissez la luminosité, activez « Assombrir légèrement l'écran sur batterie », fermez les apps en arrière-plan listées ci-dessous et débranchez les périphériques USB inutiles.",
            )
            .act("pmset:battery:lessbright:1", "Assombrir sur batterie"),
        );
    }
    if data.samples.is_empty() {
        tips.push(tip("collecting", "info", "Collecte en cours", "La consommation par app est mesurée par ThermoPilot lorsque le Mac est sur batterie. Les chiffres s'affinent au fil des heures (historique conservé 7 jours)."));
    }

    UsageReport {
        generated_at: t_now,
        recording_since: data.samples.first().map(|s| s.t),
        log_since: events.first().map(|e| e.t),
        verdict,
        checks,
        days,
        sessions,
        sleeps,
        wake_reasons,
        apps_24h,
        apps_7d,
        total_24h_wh,
        total_7d_wh,
        other_24h_wh,
        other_7d_wh,
        battery_hours_24h: r2(bh24),
        battery_hours_7d: r2(bh7),
        blockers,
        timeline,
        tips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_and_charge() {
        assert_eq!(parse_time("1970-01-01 09:00:00 +0900"), Some(0.0));
        assert_eq!(parse_charge("x Using BATT (Charge:80%) 16 secs"), Some((false, 80.0)));
        assert_eq!(parse_charge("Summary- [..] Using AC(Charge: 54)"), Some((true, 54.0)));
        assert_eq!(parse_charge("Summary- [..] Using Batt(Charge: 8          "), None);
    }

    #[test]
    fn assertions() {
        let t = "Listed by owning process:\n   pid 588(powerd): [0x1] 02:16:05 PreventUserIdleSystemSleep named: \"x\"\n   pid 72596(Slack): [0x00036bda00018907] 00:05:04 NoIdleSleepAssertion named: \"Electron\"  \nKernel Assertions: 0x100";
        let b = parse_assertions(t);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].name, "Slack");
        assert_eq!(b[0].since, "00:05:04");
    }
}
