//! Modes de contrôle, courbes de ventilation, sécurité thermique et conseiller.

use crate::fans::Fan;
use crate::i18n::{tr, trs};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mode {
    /// Contrôle Apple (thermalmonitord)
    Auto,
    /// Vitesse fixe en pourcentage (0 = min, 100 = max) appliquée à chaque ventilateur
    Manual { percent: f64 },
    /// Courbe température -> vitesse
    Curve { preset: Preset },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Silent,
    Balanced,
    Performance,
}

impl Preset {
    /// Points (°C, % entre min et max)
    fn points(self) -> &'static [(f64, f64)] {
        match self {
            Preset::Silent => &[(70.0, 0.0), (85.0, 35.0), (95.0, 100.0)],
            Preset::Balanced => &[(55.0, 0.0), (70.0, 25.0), (82.0, 60.0), (92.0, 100.0)],
            Preset::Performance => &[(45.0, 15.0), (60.0, 40.0), (75.0, 80.0), (85.0, 100.0)],
        }
    }

    pub fn percent_for(self, temp: f64) -> f64 {
        interp(self.points(), temp)
    }
}

fn interp(pts: &[(f64, f64)], t: f64) -> f64 {
    if t <= pts[0].0 {
        return pts[0].1;
    }
    for w in pts.windows(2) {
        let ((t0, p0), (t1, p1)) = (w[0], w[1]);
        if t <= t1 {
            return p0 + (p1 - p0) * (t - t0) / (t1 - t0);
        }
    }
    pts[pts.len() - 1].1
}

pub fn rpm_for_percent(fan: &Fan, pct: f64) -> f64 {
    (fan.min + (fan.max - fan.min) * pct.clamp(0.0, 100.0) / 100.0).round()
}

// ---------- Sécurité ----------

pub const SAFETY_ON: f64 = 100.0;
pub const SAFETY_OFF: f64 = 92.0;

// ---------- Conseiller ----------

#[derive(Clone, Serialize)]
pub struct Advice {
    /// cool | normal | warm | hot | critical
    pub level: &'static str,
    /// none | increase | decrease | max
    pub action: &'static str,
    pub title: String,
    pub message: String,
    /// Pourcentage de ventilation conseillé (0-100)
    pub suggested_percent: f64,
    /// RPM conseillés par ventilateur
    pub suggested_rpm: Vec<f64>,
}

pub fn advise(hotspot: f64, trend: f64, fans: &[Fan], mode: &Mode) -> Advice {
    let fan_pct = if fans.is_empty() { 0.0 } else { fans.iter().map(|f| f.percent).sum::<f64>() / fans.len() as f64 };
    let rising = trend > 8.0 && hotspot > 72.0;

    let mut level_idx = match hotspot {
        t if t < 65.0 => 0,
        t if t < 80.0 => 1,
        t if t < 90.0 => 2,
        t if t < 98.0 => 3,
        _ => 4,
    };
    if rising && level_idx < 4 {
        level_idx += 1;
    }
    let level = ["cool", "normal", "warm", "hot", "critical"][level_idx];

    // Ventilation idéale : courbe « équilibrée », renforcée si la température grimpe vite.
    let mut ideal = Preset::Balanced.percent_for(hotspot);
    if rising {
        ideal = (ideal + 20.0).min(100.0);
    }
    if level_idx == 4 {
        ideal = 100.0;
    }
    ideal = ideal.round();
    let suggested_rpm = fans.iter().map(|f| rpm_for_percent(f, ideal)).collect();

    let gap = ideal - fan_pct;
    let trend_txt = if trend.abs() >= 1.0 { tr!(" (tendance {:+.0} °C/min)", " (trend {:+.0} °C/min)", trend) } else { String::new() };
    let auto_note = if *mode == Mode::Auto {
        trs!(" En mode Auto, macOS laisse la puce monter vers ~100 °C avant d'accélérer.", " In Auto mode, macOS lets the chip reach ~100 °C before spinning up.")
    } else {
        ""
    };

    let (action, title, message) = match level_idx {
        4 => (
            "max",
            tr!("Critique : ventilation maximale", "Critical: maximum cooling"),
            tr!("Point chaud à {:.0} °C{}. La puce va brider ses performances (throttling). Passez les ventilateurs au maximum.", "Hotspot at {:.0} °C{}. The chip is about to throttle. Set the fans to maximum.", hotspot, trend_txt),
        ),
        3 if gap > 10.0 => (
            "increase",
            tr!("Chaud : augmentez la ventilation", "Hot: increase cooling"),
            tr!("Point chaud à {:.0} °C{} alors que les ventilateurs sont à {:.0} %. Montez à ~{:.0} % pour limiter le throttling et la chaleur du châssis.{}", "Hotspot at {:.0} °C{} with fans at {:.0} %. Go to ~{:.0} % to limit throttling and chassis heat.{}", hotspot, trend_txt, fan_pct, ideal, auto_note),
        ),
        2 if gap > 15.0 => (
            "increase",
            tr!("Tiède : une hausse est conseillée", "Warm: more cooling recommended"),
            tr!("Point chaud à {:.0} °C{}. Passer à ~{:.0} % (actuellement {:.0} %) garderait la machine plus fraîche sous charge prolongée.{}", "Hotspot at {:.0} °C{}. Going to ~{:.0} % (currently {:.0} %) would keep the machine cooler under sustained load.{}", hotspot, trend_txt, ideal, fan_pct, auto_note),
        ),
        // En mode Courbe, le profil a été choisi volontairement : on ne conseille pas de baisser.
        0 | 1 if gap < -30.0 && !matches!(mode, Mode::Curve { .. }) => (
            "decrease",
            tr!("Vous pouvez réduire la ventilation", "You can reduce cooling"),
            tr!("Point chaud à {:.0} °C seulement : les ventilateurs à {:.0} % font du bruit inutilement. ~{:.0} % suffit, ou repassez en Auto.", "Hotspot only at {:.0} °C: fans at {:.0} % are making needless noise. ~{:.0} % is enough, or switch back to Auto.", hotspot, fan_pct, ideal),
        ),
        _ => (
            "none",
            match level_idx {
                0 => tr!("Frais : rien à faire", "Cool: nothing to do"),
                1 => tr!("Normal : ventilation adaptée", "Normal: cooling is adequate"),
                _ => tr!("Ventilation adaptée", "Cooling is adequate"),
            },
            tr!("Point chaud à {:.0} °C{}, ventilateurs à {:.0} %. Aucune action nécessaire.", "Hotspot at {:.0} °C{}, fans at {:.0} %. No action needed.", hotspot, trend_txt, fan_pct),
        ),
    };

    Advice { level, action, title, message, suggested_percent: ideal, suggested_rpm }
}
