//! Réglages d'alimentation macOS (pmset) + réveil/veille (IORegisterForSystemPower).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::process::Command;

/// Clés pmset que l'app peut lire/modifier, avec bornes autorisées.
pub const KEYS: &[(&str, i64, i64)] = &[
    ("powermode", 0, 2),     // 0 auto, 1 économie d'énergie, 2 haute performance
    ("lowpowermode", 0, 1),  // anciens Mac
    ("powernap", 0, 1),
    ("displaysleep", 0, 180),
    ("sleep", 0, 180),
    ("lessbright", 0, 1),
    ("tcpkeepalive", 0, 1), // garde le réseau actif en veille (Localiser, iCloud…)
    ("womp", 0, 1),         // réveil par le réseau
];

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct PmsetProfiles {
    pub battery: BTreeMap<String, i64>,
    pub ac: BTreeMap<String, i64>,
}

pub fn read() -> Option<PmsetProfiles> {
    let out = Command::new("/usr/bin/pmset").args(["-g", "custom"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut p = PmsetProfiles::default();
    let mut section: Option<&mut BTreeMap<String, i64>> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Battery Power") {
            section = Some(&mut p.battery);
            continue;
        }
        if t.starts_with("AC Power") {
            section = Some(&mut p.ac);
            continue;
        }
        let mut it = t.split_whitespace();
        if let (Some(k), Some(v), Some(sec)) = (it.next(), it.next(), section.as_deref_mut()) {
            if KEYS.iter().any(|(kk, _, _)| *kk == k) {
                if let Ok(v) = v.parse() {
                    sec.insert(k.to_string(), v);
                }
            }
        }
    }
    Some(p)
}

pub fn set(profile: &str, key: &str, value: i64) -> Result<(), String> {
    let flag = match profile {
        "battery" => "-b",
        "ac" => "-c",
        _ => return Err("profil invalide".into()),
    };
    let Some(&(_, lo, hi)) = KEYS.iter().find(|(k, _, _)| *k == key) else {
        return Err(format!("réglage non autorisé : {key}"));
    };
    if value < lo || value > hi {
        return Err(format!("{key} : valeur hors bornes ({lo}-{hi})"));
    }
    let out = Command::new("/usr/bin/pmset").args([flag, key, &value.to_string()]).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("pmset : {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Réglages « autonomie maximale » pour le profil batterie.
pub fn battery_saver_preset(current: &BTreeMap<String, i64>) -> Vec<(&'static str, i64)> {
    let mut v = vec![("powernap", 0), ("displaysleep", 5), ("sleep", 15), ("lessbright", 1)];
    if current.contains_key("powermode") {
        v.push(("powermode", 1));
    } else if current.contains_key("lowpowermode") {
        v.push(("lowpowermode", 1));
    }
    v
}

// ---------- Notifications de veille ----------

pub mod sleep {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut *mut c_void,
            callback: extern "C" fn(*mut c_void, u32, u32, *mut c_void),
            notifier: *mut u32,
        ) -> u32;
        fn IONotificationPortGetRunLoopSource(port: *mut c_void) -> *mut c_void;
        fn IOAllowPowerChange(kernel_port: u32, notification_id: isize) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopRun();
        static kCFRunLoopCommonModes: *const c_void;
    }

    const CAN_SLEEP: u32 = 0xe0000270;
    const WILL_SLEEP: u32 = 0xe0000280;
    const HAS_POWERED_ON: u32 = 0xe0000300;

    static ROOT_PORT: AtomicU32 = AtomicU32::new(0);

    struct Ctx {
        on_sleep: Box<dyn Fn() + Send>,
        on_wake: Box<dyn Fn() + Send>,
    }

    extern "C" fn callback(refcon: *mut c_void, _service: u32, msg: u32, arg: *mut c_void) {
        let ctx = unsafe { &*(refcon as *const Ctx) };
        match msg {
            CAN_SLEEP => unsafe {
                IOAllowPowerChange(ROOT_PORT.load(Ordering::SeqCst), arg as isize);
            },
            WILL_SLEEP => {
                (ctx.on_sleep)();
                unsafe { IOAllowPowerChange(ROOT_PORT.load(Ordering::SeqCst), arg as isize) };
            }
            HAS_POWERED_ON => (ctx.on_wake)(),
            _ => {}
        }
    }

    /// Lance un thread qui appelle `on_sleep` juste avant la veille et `on_wake` au réveil.
    pub fn watch(on_sleep: impl Fn() + Send + 'static, on_wake: impl Fn() + Send + 'static) {
        std::thread::spawn(move || unsafe {
            let ctx = Box::into_raw(Box::new(Ctx { on_sleep: Box::new(on_sleep), on_wake: Box::new(on_wake) }));
            let mut port: *mut c_void = std::ptr::null_mut();
            let mut notifier = 0u32;
            let root = IORegisterForSystemPower(ctx as *mut c_void, &mut port, callback, &mut notifier);
            if root == 0 {
                return;
            }
            ROOT_PORT.store(root, Ordering::SeqCst);
            CFRunLoopAddSource(CFRunLoopGetCurrent(), IONotificationPortGetRunLoopSource(port), kCFRunLoopCommonModes);
            CFRunLoopRun();
        });
    }
}
