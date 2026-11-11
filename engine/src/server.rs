//! Serveur JSON-lines sur socket Unix.
//!   -> {"cmd":"status"}
//!   -> {"cmd":"set_mode","mode":{"kind":"manual","percent":60}}
//!   -> {"cmd":"set_mode","mode":{"kind":"curve","preset":"balanced"}}
//!   -> {"cmd":"set_mode","mode":{"kind":"auto"}}

use crate::charge::ChargeConfig;
use crate::control::Mode;
use crate::power;
use std::sync::atomic::Ordering;
use crate::engine::Shared;
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Ping,
    Status,
    Usage,
    SetMode { mode: Mode },
    SetCharge { config: ChargeConfig },
    SetPmset { profile: String, key: String, value: i64 },
    PowerPreset { name: String },
}

pub fn serve(path: &str, shared: Arc<Shared>, allow_uid: Option<u32>) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let listener = UnixListener::bind(path)?;
    let perms = if shared.privileged { 0o666 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(perms))?;
    eprintln!("fanengine: écoute sur {path} (root: {})", shared.privileged);

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if let Some(allowed) = allow_uid {
            let (mut uid, mut gid) = (0u32, 0u32);
            let ok = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == 0;
            if !ok || (uid != 0 && uid != allowed) {
                continue;
            }
        }
        let shared = shared.clone();
        std::thread::spawn(move || handle(stream, shared));
    }
    Ok(())
}

/// Compte la connexion tant qu'elle est ouverte ; à la fermeture, mémorise l'heure si c'était la dernière.
struct ClientGuard(Arc<Shared>);

impl ClientGuard {
    fn new(shared: Arc<Shared>) -> Self {
        shared.clients.fetch_add(1, Ordering::SeqCst);
        shared.wake_now(); // instantané frais pour le nouveau client
        ClientGuard(shared)
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        if self.0.clients.fetch_sub(1, Ordering::SeqCst) == 1 {
            *self.0.last_client_left.lock().unwrap() = Some(std::time::Instant::now());
            self.0.wake_now();
        }
    }
}

fn handle(stream: UnixStream, shared: Arc<Shared>) {
    let _guard = ClientGuard::new(shared.clone());
    let Ok(mut writer) = stream.try_clone() else { return };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Ping) => r#"{"ok":true,"data":"pong"}"#.to_string(),
            Ok(Request::Status) => {
                // Passage en « fenêtre visible » : le moteur repasse tout de suite à 1 s
                let was_active = shared.ui_active();
                shared.note_status();
                if !was_active && shared.ui_active() {
                    shared.wake_now();
                }
                format!(r#"{{"ok":true,"data":{}}}"#, shared.snapshot.lock().unwrap())
            }
            Ok(Request::Usage) => {
                let was_wanted = shared.usage_wanted();
                *shared.last_usage_req.lock().unwrap() = Some(std::time::Instant::now());
                if !was_wanted || shared.usage.lock().unwrap().as_str() == "null" {
                    shared.wake_now();
                }
                format!(r#"{{"ok":true,"data":{}}}"#, shared.usage.lock().unwrap())
            }
            Ok(Request::SetMode { mode }) => {
                if !shared.privileged && mode != Mode::Auto {
                    err("Le contrôle des ventilateurs nécessite le helper root (bouton « Installer le helper »).")
                } else {
                    let mode = match mode {
                        Mode::Manual { percent } => Mode::Manual { percent: percent.clamp(0.0, 100.0) },
                        m => m,
                    };
                    shared.set_mode(mode);
                    r#"{"ok":true,"data":null}"#.to_string()
                }
            }
            Ok(Request::SetCharge { .. } | Request::SetPmset { .. } | Request::PowerPreset { .. }) if !shared.privileged => {
                err("Cette action nécessite le helper root (bouton « Installer le helper »).")
            }
            Ok(Request::SetCharge { mut config }) => {
                config.limit = config.limit.clamp(50, 100);
                shared.settings.lock().unwrap().charge = config;
                shared.save();
                shared.wake_now();
                ok()
            }
            Ok(Request::SetPmset { profile, key, value }) => {
                let r = power::set(&profile, &key, value);
                shared.pmset_dirty.store(true, Ordering::SeqCst);
                shared.wake_now();
                r.map(|_| ok()).unwrap_or_else(|e| err(&e))
            }
            Ok(Request::PowerPreset { name }) => {
                let r = preset(&shared, &name);
                shared.pmset_dirty.store(true, Ordering::SeqCst);
                shared.wake_now();
                r.map(|_| ok()).unwrap_or_else(|e| err(&e))
            }
            Err(e) => err(&format!("requête invalide : {e}")),
        };
        if writer.write_all(resp.as_bytes()).is_err() || writer.write_all(b"\n").is_err() {
            break;
        }
    }
}

fn err(msg: &str) -> String {
    serde_json::json!({"ok": false, "error": msg}).to_string()
}

fn ok() -> String {
    r#"{"ok":true,"data":null}"#.to_string()
}

/// "battery_saver" : sauvegarde les réglages batterie actuels puis applique le profil autonomie.
/// "restore" : remet les réglages sauvegardés.
fn preset(shared: &Shared, name: &str) -> Result<(), String> {
    let current = power::read().ok_or("lecture pmset impossible")?.battery;
    match name {
        "battery_saver" => {
            {
                let mut s = shared.settings.lock().unwrap();
                if s.pmset_backup.is_none() {
                    s.pmset_backup = Some(current.clone());
                }
            }
            shared.save();
            for (k, v) in power::battery_saver_preset(&current) {
                power::set("battery", k, v)?;
            }
            Ok(())
        }
        "restore" => {
            let backup = shared.settings.lock().unwrap().pmset_backup.clone().ok_or("aucune sauvegarde à restaurer")?;
            for (k, v) in &backup {
                power::set("battery", k, *v)?;
            }
            shared.settings.lock().unwrap().pmset_backup = None;
            shared.save();
            Ok(())
        }
        _ => Err(format!("préréglage inconnu : {name}")),
    }
}
