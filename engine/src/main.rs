mod battery;
mod battery_advice;
mod charge;
mod control;
mod energy;
mod power;
mod engine;
mod fans;
mod sensors;
mod server;
mod smc;
mod usage;

use engine::{Engine, Shared, STOP};
use std::sync::atomic::Ordering;

const USAGE: &str = "fanengine — moteur thermique pour Mac Apple Silicon

USAGE:
  fanengine serve [--socket PATH] [--allow-uid UID] [--exit-with-parent]
  fanengine status            Instantané JSON (lecture seule)
  fanengine usage             Rapport d'utilisation batterie (JSON)
  fanengine dump              Toutes les clés SMC lisibles
  fanengine battery           État batterie + limiteur (JSON)
  fanengine set <0-100>       (root) Force tous les ventilateurs à un pourcentage
  fanengine auto              (root) Rend le contrôle à macOS";

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let smc = match smc::Smc::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Erreur : {e}");
            std::process::exit(1);
        }
    };
    let root = unsafe { libc::geteuid() } == 0;

    match args.first().map(String::as_str) {
        Some("serve") => {
            let socket = arg("--socket").unwrap_or_else(|| "/var/run/fanengine.sock".into());
            let allow_uid = arg("--allow-uid").and_then(|s| s.parse().ok());
            let exit_with_parent = args.iter().any(|a| a == "--exit-with-parent");
            unsafe {
                libc::signal(libc::SIGTERM, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
                libc::signal(libc::SIGINT, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
                libc::signal(libc::SIGHUP, on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t);
                libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            }
            let shared = Shared::new(root);
            let engine = Engine::new(smc);
            if root {
                // Avant la veille : réactive l'adaptateur (sinon le Mac se viderait branché, endormi)
                let (smc_h, caps) = engine.sleep_handles();
                let s_wake = shared.clone();
                power::sleep::watch(
                    move || {
                        let _ = caps.set_adapter_off(&smc_h, false);
                    },
                    move || s_wake.woke.store(true, Ordering::SeqCst),
                );
            }
            let s2 = shared.clone();
            std::thread::spawn(move || {
                if let Err(e) = server::serve(&socket, s2, allow_uid) {
                    eprintln!("fanengine: socket {socket} : {e}");
                    STOP.store(true, Ordering::SeqCst);
                }
            });
            engine.run(shared, exit_with_parent);
        }
        Some("status") => {
            let shared = Shared::new(false);
            let mut engine = Engine::new(smc);
            engine.always_full = true;
            std::thread::sleep(std::time::Duration::from_millis(500));
            let snap = engine.tick(&shared);
            println!("{}", serde_json::to_string_pretty(&snap).unwrap());
        }
        Some("battery") => {
            let shared = Shared::new(false);
            let mut engine = Engine::new(smc);
            engine.always_full = true;
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let snap = engine.tick(&shared);
            let v = serde_json::json!({
                "battery": snap.battery, "charge": snap.charge, "power_settings": snap.power_settings,
                "top_processes": snap.top_processes, "battery_tips": snap.battery_tips, "conflicts": snap.conflicts,
            });
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
        }
        Some("usage") => {
            let shared = Shared::new(false);
            let mut engine = Engine::new(smc);
            engine.always_full = true;
            engine.usage.refresh_blocking();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let snap = engine.tick(&shared);
            engine.publish_usage(&shared, snap.battery.as_ref());
            println!("{}", shared.usage.lock().unwrap());
        }
        Some("bench") => {
            use std::time::Instant;
            let n = 30;
            let cpu_us = || unsafe {
                let mut r: libc::rusage = std::mem::zeroed();
                libc::getrusage(libc::RUSAGE_SELF, &mut r);
                (r.ru_utime.tv_sec + r.ru_stime.tv_sec) as f64 * 1e6 + (r.ru_utime.tv_usec + r.ru_stime.tv_usec) as f64
            };
            let t = |label: &str, f: &mut dyn FnMut()| {
                let st = Instant::now();
                let c0 = cpu_us();
                for _ in 0..n { f(); }
                println!("{label:<22} {:>8.1} µs mur  {:>8.1} µs CPU", st.elapsed().as_secs_f64() * 1e6 / n as f64, (cpu_us() - c0) / n as f64);
            };
            let map = sensors::SensorMap::discover(&smc);
            let mut cpu = sensors::CpuLoad::new();
            let mut en = energy::EnergyTracker::new();
            t("sensors.read", &mut || { let _ = map.read(&smc); });
            t("fans.read_all", &mut || { let _ = fans::read_all(&smc); });
            t("battery.read", &mut || { let _ = battery::read(); });
            t("gpu_usage", &mut || { let _ = sensors::gpu_usage(); });
            t("cpu.sample", &mut || { let _ = cpu.sample(); });
            t("PSTR", &mut || { let _ = smc.read_f64("PSTR"); });
            t("energy.sample", &mut || en.sample());
            t("pmset -g custom", &mut || { let _ = power::read(); });
            t("pgrep", &mut || { let _ = std::process::Command::new("/usr/bin/pgrep").args(["-if", "aldente"]).output(); });
            let shared = Shared::new(false);
            let mut engine = Engine::new(smc);
            engine.always_full = true;
            let snap = engine.tick(&shared);
            t("tick total", &mut || { let _ = engine.tick(&shared); });
            t("snapshot json", &mut || { let _ = serde_json::to_string(&snap); });
            println!("snapshot json = {} octets", serde_json::to_string(&snap).unwrap().len());
        }
        Some("dump") => {
            for k in smc.all_keys() {
                let t = smc.type_of(&k).unwrap_or_default();
                match smc.read_f64(&k) {
                    Ok(v) => println!("{k} [{t}] = {v:.2}"),
                    Err(_) => println!("{k} [{t}]"),
                }
            }
        }
        Some("set") => {
            let pct: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("{USAGE}");
                std::process::exit(2)
            });
            let engine = Engine::new(smc);
            for f in fans::read_all(engine.smc()) {
                let rpm = control::rpm_for_percent(&f, pct);
                match fans::set_rpm(engine.smc(), f.id, rpm) {
                    Ok(()) => println!("{} -> {rpm} RPM", f.name),
                    Err(e) => eprintln!("{} : {e}", f.name),
                }
            }
        }
        Some("auto") => match fans::restore_auto(&smc) {
            Ok(()) => println!("Contrôle rendu à macOS."),
            Err(e) => eprintln!("Erreur : {e}"),
        },
        _ => println!("{USAGE}"),
    }
}
