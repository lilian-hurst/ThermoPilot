//! Lecture et pilotage des ventilateurs (Apple Silicon : clés F{n}Ac/Mn/Mx/Tg/Md + Ftst).

use crate::smc::{Smc, SmcError};
use serde::Serialize;
use std::thread::sleep;
use std::time::Duration;

#[derive(Clone, Serialize)]
pub struct Fan {
    pub id: usize,
    pub name: String,
    pub rpm: f64,
    pub min: f64,
    pub max: f64,
    pub target: f64,
    pub manual: bool,
    /// Position entre min (0 %) et max (100 %)
    pub percent: f64,
}

pub fn count(smc: &Smc) -> usize {
    smc.read_f64("FNum").map(|v| v as usize).unwrap_or(0)
}

/// Bornes (min, max) de chaque ventilateur : constantes, lues une seule fois.
pub fn limits(smc: &Smc) -> Vec<(f64, f64)> {
    (0..count(smc))
        .map(|i| (smc.read_f64(&format!("F{i}Mn")).unwrap_or(0.0), smc.read_f64(&format!("F{i}Mx")).unwrap_or(6000.0)))
        .collect()
}

pub fn read_all(smc: &Smc) -> Vec<Fan> {
    read_with(smc, &limits(smc))
}

/// Lecture courante : 3 clés SMC par ventilateur (vitesse, consigne, mode).
pub fn read_with(smc: &Smc, limits: &[(f64, f64)]) -> Vec<Fan> {
    limits
        .iter()
        .enumerate()
        .filter_map(|(i, &(min, max))| {
            let rpm = smc.read_f64(&format!("F{i}Ac")).ok()?;
            let target = smc.read_f64(&format!("F{i}Tg")).unwrap_or(rpm);
            // F{n}Md : 0 = auto (macOS), 1 = forcé, 3 = arrêté par macOS (ventilateur coupé au repos)
            let md = smc.read_f64(&format!("F{i}Md")).unwrap_or(0.0) as u8;
            let manual = md == 1;
            let percent = if max > min { ((rpm - min) / (max - min) * 100.0).clamp(0.0, 100.0) } else { 0.0 };
            Some(Fan {
                id: i,
                name: format!("Ventilateur {}", i + 1),
                rpm: rpm.round(),
                min: min.round(),
                max: max.round(),
                target: target.round(),
                manual,
                percent: percent.round(),
            })
        })
        .collect()
}

/// Passe le ventilateur en mode forcé puis fixe la consigne.
/// Sur M3/M4, `Ftst=1` est nécessaire pour que thermalmonitord relâche le contrôle.
pub fn set_rpm(smc: &Smc, id: usize, rpm: f64) -> Result<(), SmcError> {
    let md = format!("F{id}Md");
    let forced = |smc: &Smc| smc.read_f64(&md).map(|v| v as u8 == 1).unwrap_or(false);
    let unlocked = |smc: &Smc| !smc.has_key("Ftst") || smc.read_f64("Ftst").unwrap_or(0.0) >= 1.0;
    if !forced(smc) || !unlocked(smc) {
        if smc.has_key("Ftst") {
            smc.write_f64("Ftst", 1.0)?;
        }
        let mut last = Ok(());
        for _ in 0..40 {
            last = smc.write_f64(&md, 1.0);
            if last.is_ok() && forced(smc) {
                break;
            }
            if last.is_ok() {
                last = Err(SmcError::Unsupported("macOS a refusé le mode forcé".into()));
            }
            sleep(Duration::from_millis(125));
        }
        last?;
    }
    smc.write_f64(&format!("F{id}Tg"), rpm)
}

/// Rend la main au système (thermalmonitord) pour tous les ventilateurs.
pub fn restore_auto(smc: &Smc) -> Result<(), SmcError> {
    let mut res = Ok(());
    for i in 0..count(smc) {
        if let Err(e) = smc.write_f64(&format!("F{i}Md"), 0.0) {
            res = Err(e);
        }
    }
    if smc.has_key("Ftst") {
        let _ = smc.write_f64("Ftst", 0.0);
    }
    res
}
