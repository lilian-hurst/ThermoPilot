//! Conseils autonomie + durée de vie de la batterie.

use crate::battery::BatteryInfo;
use crate::charge::ChargeStatus;
use crate::control::Mode;
use crate::energy::ProcEnergy;
use crate::power::PmsetProfiles;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct Tip {
    pub id: &'static str,
    /// good | info | warn | bad
    pub level: &'static str,
    pub title: String,
    pub message: String,
    /// Action interprétée par l'app (voir README)
    pub action: Option<String>,
    pub action_label: Option<String>,
}

pub fn tip(id: &'static str, level: &'static str, title: impl Into<String>, message: impl Into<String>) -> Tip {
    Tip { id, level, title: title.into(), message: message.into(), action: None, action_label: None }
}

impl Tip {
    pub fn act(mut self, action: impl Into<String>, label: impl Into<String>) -> Tip {
        self.action = Some(action.into());
        self.action_label = Some(label.into());
        self
    }
}

pub struct Ctx<'a> {
    pub bat: &'a BatteryInfo,
    pub charge: &'a ChargeStatus,
    pub pmset: Option<&'a PmsetProfiles>,
    pub top: &'a [ProcEnergy],
    pub fan_mode: &'a Mode,
    pub conflicts: &'a [String],
    pub gpu_usage: Option<f64>,
    pub privileged: bool,
}

pub fn tips(c: &Ctx) -> Vec<Tip> {
    let b = c.bat;
    let mut v = Vec::new();
    let on_battery = !b.external_connected;
    let cfg = &c.charge.config;
    let can_limit = c.charge.method != "none";
    let bp = c.pmset.map(|p| &p.battery);

    // ---------- Conflits ----------
    if !c.conflicts.is_empty() {
        v.push(tip(
            "conflict",
            "warn",
            "Autre gestionnaire de charge détecté",
            format!(
                "{} modifie aussi la charge. Deux outils qui écrivent les mêmes clés SMC se contredisent : gardez-en un seul (désactivez l'autre ou le limiteur de ThermoPilot).",
                c.conflicts.join(" et ")
            ),
        ));
    }

    // ---------- Durée de vie ----------
    if b.health_pct > 0.0 && b.health_pct < 80.0 {
        v.push(tip("health_bad", "bad", format!("Batterie usée ({:.0} %)", b.health_pct),
            "Sous 80 % de la capacité d'origine, Apple considère que la batterie doit être remplacée (Réglages > Batterie > Santé)."));
    } else if b.health_pct >= 80.0 && b.health_pct < 90.0 {
        v.push(tip("health_mid", "info", format!("Santé {:.0} %", b.health_pct),
            "Usure normale. Pour la ralentir : limiter la charge à 80 %, éviter la chaleur et les décharges sous 20 %."));
    }

    if b.external_connected && !cfg.enabled && b.percent >= 90.0 {
        let t = tip("limit", "warn", "Branché en permanence à 100 % : usure accélérée",
            "Une batterie Li-ion stockée pleine et chaude vieillit beaucoup plus vite. Limiter la charge à 80 % quand le Mac reste sur secteur double environ sa durée de vie en cycles.");
        v.push(if can_limit && c.privileged {
            t.act("enable_limit", "Limiter à 80 %")
        } else if can_limit {
            t.act("install_helper", "Installer le helper")
        } else {
            t.act("open_battery_settings", "Recharge optimisée…")
        });
    }

    if c.charge.effective == Some(false) {
        v.push(tip("limit_ineffective", "bad", "Le firmware semble ignorer le limiteur",
            format!("Après la commande ({}), le courant de la batterie n'a pas changé. Ce Mac utilise peut-être des clés différentes : repassez le limiteur sur Off et utilisez la « Recharge optimisée » de macOS.",
                c.charge.inhibit_key.or(c.charge.adapter_key).unwrap_or("?")))
            .act("open_battery_settings", "Réglages Batterie"));
    } else if c.charge.method == "sailing" && cfg.enabled {
        v.push(tip("sailing", "info", "Limiteur en mode « navigation »",
            "Ce firmware n'expose pas de clé de pause de charge : à la limite, l'adaptateur est coupé et le Mac tourne sur batterie jusqu'à 5 points sous la limite, puis recharge. Pendant la veille, l'adaptateur est réactivé par sécurité."));
    }

    if b.temperature_c >= 40.0 {
        v.push(tip("hot_bad", "bad", format!("Batterie très chaude ({:.0} °C)", b.temperature_c),
            "Au-delà de 40 °C l'usure s'accélère fortement. Aérez le Mac (pas sur un lit/canapé), augmentez la ventilation et évitez de charger maintenant.")
            .act("fans_boost", "Ventilation Performance"));
    } else if b.temperature_c >= 35.0 && (b.is_charging || b.external_connected) {
        let t = tip("hot_charge", "warn", format!("Charge à chaud ({:.0} °C)", b.temperature_c),
            "Charger au-dessus de 35 °C use la batterie. La protection thermique met la charge en pause le temps que ça refroidisse.");
        v.push(if !cfg.heat_protection && can_limit && c.privileged { t.act("heat_protection", "Activer la protection") } else { t.act("fans_boost", "Ventilation Performance") });
    }

    if on_battery && b.percent <= 20.0 {
        v.push(tip("low", "warn", format!("Batterie à {:.0} %", b.percent),
            "Les décharges profondes (sous 20 %) stressent la batterie. Branchez le Mac dès que possible."));
    }

    let wear = b.cycles as f64 / b.design_cycles.max(1) as f64;
    if wear >= 0.8 {
        v.push(tip("cycles", "info", format!("{} cycles sur {}", b.cycles, b.design_cycles),
            "La batterie approche de sa durée de vie nominale. Limiter la charge à 80 % réduit le nombre de cycles consommés."));
    }

    // ---------- Autonomie (réglages macOS) ----------
    if let Some(bp) = bp {
        if bp.get("powermode") == Some(&2) {
            v.push(tip("highpower", if on_battery { "bad" } else { "warn" }, "Mode « Haute performance » actif sur batterie",
                "Sur batterie, ce mode garde le CPU/GPU à fond et les ventilateurs plus rapides : l'autonomie peut chuter de moitié. « Automatique » suffit dans la plupart des cas.")
                .act("pmset:battery:powermode:0", "Passer en Automatique"));
        }
        if bp.get("displaysleep") == Some(&0) {
            v.push(tip("displaysleep", "warn", "L'écran ne s'éteint jamais sur batterie",
                "L'écran est le premier poste de consommation (souvent 20 à 30 %). Éteignez-le après 5 min d'inactivité.")
                .act("pmset:battery:displaysleep:5", "Éteindre après 5 min"));
        }
        if bp.get("powernap") == Some(&1) {
            v.push(tip("powernap", "info", "Power Nap actif sur batterie",
                "Le Mac se réveille pendant la veille pour synchroniser mails, iCloud et sauvegardes, ce qui vide la batterie.")
                .act("pmset:battery:powernap:0", "Désactiver"));
        }
        if bp.get("sleep") == Some(&0) {
            v.push(tip("sleep", "warn", "La mise en veille est désactivée sur batterie",
                "Sans veille automatique, un Mac oublié ouvert se vide complètement.")
                .act("pmset:battery:sleep:15", "Veille après 15 min"));
        }
    }

    if on_battery && !matches!(c.fan_mode, Mode::Auto) {
        v.push(tip("fans", "info", "Ventilateurs forcés sur batterie",
            "Les ventilateurs consomment jusqu'à quelques watts à haut régime. En Auto, macOS les coupe quand la machine est fraîche.")
            .act("fans_auto", "Ventilateurs en Auto"));
    }

    // ---------- Autonomie (usage) ----------
    if on_battery && b.power_w < -25.0 {
        let gpu = c.gpu_usage.filter(|g| *g > 40.0).map(|g| format!(" Le GPU est à {:.0} %.", g)).unwrap_or_default();
        v.push(tip("drain", "warn", format!("Consommation élevée ({:.0} W)", -b.power_w),
            format!("À ce rythme la batterie tient environ {:.1} h.{gpu} Fermez les apps lourdes ou activez l'économie d'énergie.",
                b.current_mah * b.voltage_v / 1000.0 / -b.power_w)));
    }
    let threshold = if on_battery { 2.0 } else { 6.0 };
    for p in c.top.iter().filter(|p| p.watts >= threshold && !is_system(&p.name)).take(2) {
        v.push(tip("process", if on_battery { "warn" } else { "info" }, format!("{} consomme ~{:.1} W", p.name, p.watts),
            format!("{:.0} % de CPU{}. Si vous ne l'utilisez pas, quittez-le pour gagner en autonomie et en chaleur.",
                p.cpu_pct, if p.count > 1 { format!(" sur {} processus", p.count) } else { String::new() }))
            .act(format!("quit:{}:{}", p.pid, p.name), "Quitter"));
    }
    if on_battery {
        v.push(tip("brightness", "info", "Luminosité et périphériques",
            "Baisser la luminosité de 100 % à 60 % fait gagner 1 à 2 h. Débranchez aussi les périphériques USB inutiles et coupez le Bluetooth si vous ne l'utilisez pas."));
    }

    if v.iter().all(|t| t.level == "info") && b.health_pct >= 90.0 && (cfg.enabled || !b.external_connected) {
        v.insert(0, tip("ok", "good", "Batterie bien gérée", format!("Santé {:.0} %, {} cycles. Continuez ainsi.", b.health_pct, b.cycles)));
    }

    let rank = |l: &str| match l { "bad" => 0, "warn" => 1, "good" => 2, _ => 3 };
    v.sort_by_key(|t| rank(t.level));
    v
}

fn is_system(name: &str) -> bool {
    matches!(name, "kernel_task" | "WindowServer" | "launchd" | "fanengine" | "com.lilian.fanengine" | "powerd" | "coreaudiod" | "mds" | "mds_stores" | "mdworker_shared")
}
