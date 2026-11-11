//! Conseils autonomie + durée de vie de la batterie.

use crate::battery::BatteryInfo;
use crate::i18n::{tr, trs};
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
        let who = c.conflicts.join(trs!(" et ", " and "));
        v.push(tip(
            "conflict",
            "warn",
            tr!("Autre gestionnaire de charge détecté", "Another charge manager detected"),
            tr!(
                "{who} modifie aussi la charge. Deux outils qui écrivent les mêmes clés SMC se contredisent : gardez-en un seul (désactivez l'autre ou le limiteur de ThermoPilot).",
                "{who} also controls charging. Two tools writing the same SMC keys fight each other: keep only one (disable the other one or ThermoPilot's limiter)."
            ),
        ));
    }

    // ---------- Durée de vie ----------
    if b.health_pct > 0.0 && b.health_pct < 80.0 {
        v.push(tip("health_bad", "bad", tr!("Batterie usée ({:.0} %)", "Worn battery ({:.0} %)", b.health_pct),
            tr!("Sous 80 % de la capacité d'origine, Apple considère que la batterie doit être remplacée (Réglages > Batterie > Santé).",
                "Below 80 % of its design capacity, Apple considers the battery due for replacement (Settings > Battery > Health).")));
    } else if b.health_pct >= 80.0 && b.health_pct < 90.0 {
        v.push(tip("health_mid", "info", tr!("Santé {:.0} %", "Health {:.0} %", b.health_pct),
            tr!("Usure normale. Pour la ralentir : limiter la charge à 80 %, éviter la chaleur et les décharges sous 20 %.",
                "Normal wear. To slow it down: limit charging to 80 %, avoid heat and discharges below 20 %.")));
    }

    if b.external_connected && !cfg.enabled && b.percent >= 90.0 {
        let t = tip("limit", "warn", tr!("Branché en permanence à 100 % : usure accélérée", "Always plugged in at 100 %: faster wear"),
            tr!("Une batterie Li-ion stockée pleine et chaude vieillit beaucoup plus vite. Limiter la charge à 80 % quand le Mac reste sur secteur double environ sa durée de vie en cycles.",
                "A Li-ion battery kept full and warm ages much faster. Limiting charge to 80 % when the Mac stays plugged in roughly doubles its cycle life."));
        v.push(if can_limit && c.privileged {
            t.act("enable_limit", tr!("Limiter à 80 %", "Limit to 80 %"))
        } else if can_limit {
            t.act("install_helper", tr!("Installer le helper", "Install helper"))
        } else {
            t.act("open_battery_settings", tr!("Recharge optimisée…", "Optimized charging…"))
        });
    }

    if c.charge.effective == Some(false) {
        let key = c.charge.inhibit_key.or(c.charge.adapter_key).unwrap_or("?");
        v.push(tip("limit_ineffective", "bad", tr!("Le firmware semble ignorer le limiteur", "The firmware seems to ignore the limiter"),
            tr!("Après la commande ({key}), le courant de la batterie n'a pas changé. Ce Mac utilise peut-être des clés différentes : repassez le limiteur sur Off et utilisez la « Recharge optimisée » de macOS.",
                "After the command ({key}), the battery current did not change. This Mac may use different keys: turn the limiter off and use macOS \"Optimized Battery Charging\"."))
            .act("open_battery_settings", tr!("Réglages Batterie", "Battery Settings")));
    } else if c.charge.method == "sailing" && cfg.enabled {
        v.push(tip("sailing", "info", tr!("Limiteur en mode « navigation »", "Limiter in \"sailing\" mode"),
            tr!("Ce firmware n'expose pas de clé de pause de charge : à la limite, l'adaptateur est coupé et le Mac tourne sur batterie jusqu'à 5 points sous la limite, puis recharge. Pendant la veille, l'adaptateur est réactivé par sécurité.",
                "This firmware has no charge-pause key: at the limit the adapter is cut and the Mac runs on battery until 5 points below the limit, then charges again. During sleep the adapter is re-enabled for safety.")));
    }

    if b.temperature_c >= 40.0 {
        v.push(tip("hot_bad", "bad", tr!("Batterie très chaude ({:.0} °C)", "Battery very hot ({:.0} °C)", b.temperature_c),
            tr!("Au-delà de 40 °C l'usure s'accélère fortement. Aérez le Mac (pas sur un lit/canapé), augmentez la ventilation et évitez de charger maintenant.",
                "Above 40 °C wear accelerates sharply. Give the Mac some air (not on a bed/couch), increase cooling and avoid charging right now."))
            .act("fans_boost", tr!("Ventilation Performance", "Performance cooling")));
    } else if b.temperature_c >= 35.0 && (b.is_charging || b.external_connected) {
        let t = tip("hot_charge", "warn", tr!("Charge à chaud ({:.0} °C)", "Charging while hot ({:.0} °C)", b.temperature_c),
            tr!("Charger au-dessus de 35 °C use la batterie. La protection thermique met la charge en pause le temps que ça refroidisse.",
                "Charging above 35 °C wears the battery. Heat protection pauses charging until it cools down."));
        v.push(if !cfg.heat_protection && can_limit && c.privileged {
            t.act("heat_protection", tr!("Activer la protection", "Enable protection"))
        } else {
            t.act("fans_boost", tr!("Ventilation Performance", "Performance cooling"))
        });
    }

    if on_battery && b.percent <= 20.0 {
        v.push(tip("low", "warn", tr!("Batterie à {:.0} %", "Battery at {:.0} %", b.percent),
            tr!("Les décharges profondes (sous 20 %) stressent la batterie. Branchez le Mac dès que possible.",
                "Deep discharges (below 20 %) stress the battery. Plug the Mac in as soon as possible.")));
    }

    let wear = b.cycles as f64 / b.design_cycles.max(1) as f64;
    if wear >= 0.8 {
        v.push(tip("cycles", "info", tr!("{} cycles sur {}", "{} of {} cycles", b.cycles, b.design_cycles),
            tr!("La batterie approche de sa durée de vie nominale. Limiter la charge à 80 % réduit le nombre de cycles consommés.",
                "The battery is nearing its rated lifespan. Limiting charge to 80 % reduces the cycles used.")));
    }

    // ---------- Autonomie (réglages macOS) ----------
    if let Some(bp) = bp {
        if bp.get("powermode") == Some(&2) {
            v.push(tip("highpower", if on_battery { "bad" } else { "warn" }, tr!("Mode « Haute performance » actif sur batterie", "\"High Power\" mode on battery"),
                tr!("Sur batterie, ce mode garde le CPU/GPU à fond et les ventilateurs plus rapides : l'autonomie peut chuter de moitié. « Automatique » suffit dans la plupart des cas.",
                    "On battery this mode keeps the CPU/GPU at full speed and the fans faster: battery life can drop by half. \"Automatic\" is enough in most cases."))
                .act("pmset:battery:powermode:0", tr!("Passer en Automatique", "Switch to Automatic")));
        }
        if bp.get("displaysleep") == Some(&0) {
            v.push(tip("displaysleep", "warn", tr!("L'écran ne s'éteint jamais sur batterie", "The display never turns off on battery"),
                tr!("L'écran est le premier poste de consommation (souvent 20 à 30 %). Éteignez-le après 5 min d'inactivité.",
                    "The display is the biggest consumer (often 20 to 30 %). Turn it off after 5 min of inactivity."))
                .act("pmset:battery:displaysleep:5", tr!("Éteindre après 5 min", "Turn off after 5 min")));
        }
        if bp.get("powernap") == Some(&1) {
            v.push(tip("powernap", "info", tr!("Power Nap actif sur batterie", "Power Nap on battery"),
                tr!("Le Mac se réveille pendant la veille pour synchroniser mails, iCloud et sauvegardes, ce qui vide la batterie.",
                    "The Mac wakes during sleep to sync mail, iCloud and backups, which drains the battery."))
                .act("pmset:battery:powernap:0", tr!("Désactiver", "Disable")));
        }
        if bp.get("sleep") == Some(&0) {
            v.push(tip("sleep", "warn", tr!("La mise en veille est désactivée sur batterie", "Sleep is disabled on battery"),
                tr!("Sans veille automatique, un Mac oublié ouvert se vide complètement.",
                    "Without automatic sleep, a Mac left open drains completely."))
                .act("pmset:battery:sleep:15", tr!("Veille après 15 min", "Sleep after 15 min")));
        }
    }

    if on_battery && !matches!(c.fan_mode, Mode::Auto) {
        v.push(tip("fans", "info", tr!("Ventilateurs forcés sur batterie", "Fans forced on battery"),
            tr!("Les ventilateurs consomment jusqu'à quelques watts à haut régime. En Auto, macOS les coupe quand la machine est fraîche.",
                "Fans draw up to a few watts at high speed. In Auto, macOS turns them off when the machine is cool."))
            .act("fans_auto", tr!("Ventilateurs en Auto", "Fans to Auto")));
    }

    // ---------- Autonomie (usage) ----------
    if on_battery && b.power_w < -25.0 {
        let gpu = c.gpu_usage.filter(|g| *g > 40.0).map(|g| tr!(" Le GPU est à {:.0} %.", " The GPU is at {:.0} %.", g)).unwrap_or_default();
        let hours = b.current_mah * b.voltage_v / 1000.0 / -b.power_w;
        v.push(tip("drain", "warn", tr!("Consommation élevée ({:.0} W)", "High power draw ({:.0} W)", -b.power_w),
            tr!("À ce rythme la batterie tient environ {hours:.1} h.{gpu} Fermez les apps lourdes ou activez l'économie d'énergie.",
                "At this rate the battery lasts about {hours:.1} h.{gpu} Close heavy apps or enable Low Power mode.")));
    }
    let threshold = if on_battery { 2.0 } else { 6.0 };
    for p in c.top.iter().filter(|p| p.watts >= threshold && !is_system(&p.name)).take(2) {
        let procs = if p.count > 1 { tr!(" sur {} processus", " across {} processes", p.count) } else { String::new() };
        v.push(tip("process", if on_battery { "warn" } else { "info" }, tr!("{} consomme ~{:.1} W", "{} draws ~{:.1} W", p.name, p.watts),
            tr!("{:.0} % de CPU{}. Si vous ne l'utilisez pas, quittez-le pour gagner en autonomie et en chaleur.",
                "{:.0} % CPU{}. If you're not using it, quit it to save battery and heat.", p.cpu_pct, procs))
            .act(format!("quit:{}:{}", p.pid, p.name), tr!("Quitter", "Quit")));
    }
    if on_battery {
        v.push(tip("brightness", "info", tr!("Luminosité et périphériques", "Brightness and peripherals"),
            tr!("Baisser la luminosité de 100 % à 60 % fait gagner 1 à 2 h. Débranchez aussi les périphériques USB inutiles et coupez le Bluetooth si vous ne l'utilisez pas.",
                "Lowering brightness from 100 % to 60 % gains 1 to 2 hours. Also unplug unused USB devices and turn off Bluetooth if you don't use it.")));
    }

    if v.iter().all(|t| t.level == "info") && b.health_pct >= 90.0 && (cfg.enabled || !b.external_connected) {
        v.insert(0, tip("ok", "good", tr!("Batterie bien gérée", "Battery well managed"),
            tr!("Santé {:.0} %, {} cycles. Continuez ainsi.", "Health {:.0} %, {} cycles. Keep it up.", b.health_pct, b.cycles)));
    }

    let rank = |l: &str| match l { "bad" => 0, "warn" => 1, "good" => 2, _ => 3 };
    v.sort_by_key(|t| rank(t.level));
    v
}

fn is_system(name: &str) -> bool {
    matches!(name, "kernel_task" | "WindowServer" | "launchd" | "fanengine" | "com.lilian.fanengine" | "powerd" | "coreaudiod" | "mds" | "mds_stores" | "mdworker_shared")
}
