//! Limiteur de charge (durée de vie de la batterie).
//!
//! Selon le firmware, deux familles de clés SMC existent :
//!  - inhibition de charge : `CHTE` (firmwares récents) ou `CH0B`/`CH0C` (anciens)
//!  - coupure de l'adaptateur (le Mac tourne sur batterie branché) : `CHIE` ou `CH0I`
//! Le moteur détecte ce qui est disponible, l'applique et VÉRIFIE l'effet réel.

use crate::battery::BatteryInfo;
use crate::smc::Smc;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inhibit {
    Chte,
    Ch0bc,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Adapter {
    Chie,
    Ch0i,
}

#[derive(Clone, Copy, Debug)]
pub struct Caps {
    pub inhibit: Option<Inhibit>,
    pub adapter: Option<Adapter>,
}

impl Caps {
    pub fn detect(smc: &Smc) -> Caps {
        let inhibit = if smc.has_key("CHTE") {
            Some(Inhibit::Chte)
        } else if smc.has_key("CH0B") && smc.has_key("CH0C") {
            Some(Inhibit::Ch0bc)
        } else {
            None
        };
        let adapter = if smc.has_key("CHIE") {
            Some(Adapter::Chie)
        } else if smc.has_key("CH0I") {
            Some(Adapter::Ch0i)
        } else {
            None
        };
        Caps { inhibit, adapter }
    }

    pub fn set_inhibit(&self, smc: &Smc, on: bool) -> Result<(), String> {
        match self.inhibit {
            Some(Inhibit::Chte) => smc.write_raw("CHTE", &[on as u8, 0, 0, 0]).map_err(|e| e.to_string()),
            Some(Inhibit::Ch0bc) => {
                let v = if on { 2 } else { 0 };
                smc.write_raw("CH0B", &[v]).and_then(|_| smc.write_raw("CH0C", &[v])).map_err(|e| e.to_string())
            }
            None => Ok(()),
        }
    }

    pub fn set_adapter_off(&self, smc: &Smc, off: bool) -> Result<(), String> {
        match self.adapter {
            Some(Adapter::Chie) => smc.write_raw("CHIE", &[if off { 0x08 } else { 0 }]).map_err(|e| e.to_string()),
            Some(Adapter::Ch0i) => smc.write_raw("CH0I", &[off as u8]).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// Remet tout à l'état normal (charge autorisée, adaptateur actif).
    pub fn reset(&self, smc: &Smc) {
        let _ = self.set_inhibit(smc, false);
        let _ = self.set_adapter_off(smc, false);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChargeConfig {
    /// Limiter la charge
    pub enabled: bool,
    /// Pourcentage maximal (50-100)
    pub limit: u8,
    /// Mettre la charge en pause si la batterie dépasse 35 °C
    pub heat_protection: bool,
    /// Si la batterie est au-dessus de la limite en étant branché, la redescendre (adaptateur coupé)
    pub discharge_to_limit: bool,
}

impl Default for ChargeConfig {
    fn default() -> Self {
        ChargeConfig { enabled: false, limit: 80, heat_protection: true, discharge_to_limit: false }
    }
}

#[derive(Clone, Serialize, Default)]
pub struct ChargeStatus {
    pub config: ChargeConfig,
    pub inhibit_key: Option<&'static str>,
    pub adapter_key: Option<&'static str>,
    /// inhibit | sailing | none
    pub method: &'static str,
    pub charging_inhibited: bool,
    pub adapter_disabled: bool,
    pub heat_paused: bool,
    pub state: String,
    /// None = pas encore vérifié, Some(false) = le firmware semble ignorer la commande
    pub effective: Option<bool>,
    pub last_error: Option<String>,
}

pub struct Limiter {
    pub caps: Caps,
    holding: bool,
    hot: bool,
    discharging_down: bool,
    applied: Option<(bool, bool)>,
    applied_at: Instant,
    last_assert: Instant,
    effective: Option<bool>,
    last_error: Option<String>,
}

impl Limiter {
    pub fn new(smc: &Smc) -> Limiter {
        Limiter {
            caps: Caps::detect(smc),
            holding: false,
            hot: false,
            discharging_down: false,
            applied: None,
            applied_at: Instant::now(),
            last_assert: Instant::now(),
            effective: None,
            last_error: None,
        }
    }

    /// À appeler après un réveil : l'état SMC a pu changer.
    pub fn invalidate(&mut self) {
        self.applied = None;
    }

    pub fn tick(&mut self, smc: &Smc, cfg: &ChargeConfig, bat: Option<&BatteryInfo>, privileged: bool) -> ChargeStatus {
        let method = match (self.caps.inhibit, self.caps.adapter) {
            (Some(_), _) => "inhibit",
            (None, Some(_)) => "sailing",
            _ => "none",
        };
        let mut status = ChargeStatus {
            config: cfg.clone(),
            inhibit_key: self.caps.inhibit.map(|i| if i == Inhibit::Chte { "CHTE" } else { "CH0B/CH0C" }),
            adapter_key: self.caps.adapter.map(|a| if a == Adapter::Chie { "CHIE" } else { "CH0I" }),
            method,
            ..Default::default()
        };
        let Some(bat) = bat else {
            status.state = "Pas de batterie".into();
            return status;
        };

        let level = bat.percent;
        let limit = cfg.limit.clamp(50, 100) as f64;
        let active = cfg.enabled && limit < 100.0;

        // Hystérésis : on tient la charge à la limite, on reprend 5 points plus bas
        if active && level >= limit {
            self.holding = true;
        } else if !active || level <= limit - 5.0 {
            self.holding = false;
        }
        if cfg.heat_protection && bat.temperature_c >= 35.0 {
            self.hot = true;
        } else if !cfg.heat_protection || bat.temperature_c < 33.0 {
            self.hot = false;
        }
        if active && cfg.discharge_to_limit && level > limit + 2.0 {
            self.discharging_down = true;
        } else if !active || !cfg.discharge_to_limit || level <= limit {
            self.discharging_down = false;
        }

        let has_inhibit = self.caps.inhibit.is_some();
        let has_adapter = self.caps.adapter.is_some();
        let safety_low = level <= 15.0;
        let mut want_inhibit = has_inhibit && (self.holding || self.hot);
        let mut want_adapter_off = has_adapter && (self.discharging_down || (!has_inhibit && (self.holding || self.hot)));
        if safety_low {
            want_inhibit = false;
            want_adapter_off = false;
        }

        if privileged && (has_inhibit || has_adapter) {
            let want = (want_inhibit, want_adapter_off);
            let reassert = self.last_assert.elapsed() > Duration::from_secs(30);
            if self.applied != Some(want) || reassert {
                let mut err = None;
                if let Err(e) = self.caps.set_inhibit(smc, want_inhibit) {
                    err = Some(format!("Inhibition de charge : {e}"));
                }
                if let Err(e) = self.caps.set_adapter_off(smc, want_adapter_off) {
                    err = Some(format!("Coupure adaptateur : {e}"));
                }
                if self.applied != Some(want) {
                    self.applied_at = Instant::now();
                    self.effective = None;
                }
                self.applied = Some(want);
                self.last_assert = Instant::now();
                self.last_error = err;
            }
            // Vérification : 25 s après une commande, le courant batterie doit avoir suivi
            if self.effective.is_none() && self.applied_at.elapsed() > Duration::from_secs(25) && bat.external_connected {
                if want_adapter_off {
                    self.effective = Some(bat.amperage_a < -0.05);
                } else if want_inhibit {
                    self.effective = Some(bat.amperage_a < 0.15);
                }
            }
        }

        status.charging_inhibited = want_inhibit;
        status.adapter_disabled = want_adapter_off;
        status.heat_paused = self.hot;
        status.effective = self.effective;
        status.last_error = self.last_error.clone();
        status.state = if !bat.external_connected {
            "Sur batterie".into()
        } else if safety_low {
            "Batterie faible : charge forcée".into()
        } else if want_adapter_off && self.discharging_down {
            format!("Décharge vers la limite de {:.0} %", limit)
        } else if (want_inhibit || want_adapter_off) && self.hot {
            format!("Charge en pause : batterie chaude ({:.0} °C)", bat.temperature_c)
        } else if want_inhibit || want_adapter_off {
            format!("Charge en pause : limite de {:.0} % atteinte", limit)
        } else if bat.is_charging {
            if active { format!("En charge jusqu'à {:.0} %", limit) } else { "En charge jusqu'à 100 %".into() }
        } else if bat.fully_charged || level >= 99.0 {
            "Chargée (secteur)".into()
        } else {
            "Branché, pas en charge".into()
        };
        status
    }

    pub fn shutdown(&self, smc: &Smc) {
        self.caps.reset(smc);
    }
}
