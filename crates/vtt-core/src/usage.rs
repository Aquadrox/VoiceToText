//! Activité par jour : dictées (nombre, mots, durée parlée) et consommation
//! du LLM (jetons, coût), pour les statistiques et le suivi du coût.
//!
//! Le fichier des totaux ne contient que des compteurs, jamais de texte
//! dicté : il survit à l'effacement de l'historique. Un second fichier
//! (`usage-apps.json`) tient les mêmes compteurs par application et par
//! mois : nom de l'exécutable seulement, jamais le titre de la fenêtre.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::Pricing;

/// Jetons facturés pour une requête.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Jetons envoyés, y compris ceux servis par le cache du fournisseur.
    pub input_tokens: u64,
    /// Part de `input_tokens` servie par le cache (facturée moins cher).
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
}

impl Pricing {
    pub fn cost(&self, usage: &Usage) -> f64 {
        let cached = usage.cached_input_tokens.min(usage.input_tokens);
        let uncached = usage.input_tokens - cached;
        (uncached as f64 * self.input_per_million
            + cached as f64 * self.cached_input_per_million
            + usage.output_tokens as f64 * self.output_per_million)
            / 1_000_000.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Totals {
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    /// Dans la devise du tarif configuré au moment de chaque requête.
    pub cost: f64,
    /// Dictées insérées (commandes comprises).
    pub dictations: u64,
    /// Commandes (Command Mode) parmi les dictées.
    pub commands: u64,
    /// Mots prononcés (transcription brute).
    pub words: u64,
    /// Durée parlée.
    pub audio_ms: u64,
}

impl Totals {
    fn add(&mut self, other: &Totals) {
        self.requests += other.requests;
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.cost += other.cost;
        self.dictations += other.dictations;
        self.commands += other.commands;
        self.words += other.words;
        self.audio_ms += other.audio_ms;
    }

    pub fn is_zero(&self) -> bool {
        self.requests == 0
            && self.input_tokens == 0
            && self.cached_input_tokens == 0
            && self.output_tokens == 0
            && self.cost == 0.0
            && self.dictations == 0
            && self.commands == 0
            && self.words == 0
            && self.audio_ms == 0
    }

    /// Fusionne un compteur monotone du même appareil (CRDT G-Counter) en
    /// prenant le maximum de chaque champ ; renvoie `true` si un champ a augmenté.
    pub fn merge_max(&mut self, other: &Totals) -> bool {
        let mut changed = false;
        macro_rules! max_u64 {
            ($field:ident) => {
                if other.$field > self.$field {
                    self.$field = other.$field;
                    changed = true;
                }
            };
        }
        max_u64!(requests);
        max_u64!(input_tokens);
        max_u64!(cached_input_tokens);
        max_u64!(output_tokens);
        max_u64!(dictations);
        max_u64!(commands);
        max_u64!(words);
        max_u64!(audio_ms);
        if other.cost.is_finite() && other.cost > self.cost {
            self.cost = other.cost;
            changed = true;
        }
        changed
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Summary {
    pub today: Totals,
    pub month: Totals,
    pub total: Totals,
}

/// Totaux d'une application.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppTotals {
    /// Nom de l'exécutable en minuscules, ex. « slack.exe ».
    pub app: String,
    pub totals: Totals,
}

/// Application → mois (`AAAA-MM`) → totaux.
type AppMonths = BTreeMap<String, BTreeMap<String, Totals>>;

/// Instantané des compteurs d'activité d'un appareil donné (sans aucun texte).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageSnapshot {
    pub days: BTreeMap<String, Totals>,
    pub apps: AppMonths,
}

impl UsageSnapshot {
    pub fn is_empty(&self) -> bool {
        self.days.values().all(Totals::is_zero)
            && self
                .apps
                .values()
                .all(|months| months.values().all(Totals::is_zero))
    }

    /// Fusionne `other` dans `self` par maximum champ à champ (idempotent et
    /// commutatif pour un même appareil) ; renvoie `true` si `self` a évolué.
    pub fn merge_max(&mut self, other: &UsageSnapshot) -> bool {
        let mut changed = false;
        for (day, totals) in &other.days {
            let day = day.trim();
            if day.is_empty() || totals.is_zero() {
                continue;
            }
            if self
                .days
                .entry(day.to_owned())
                .or_default()
                .merge_max(totals)
            {
                changed = true;
            }
        }
        for (app, months) in &other.apps {
            let app = app.trim().to_lowercase();
            if app.is_empty() {
                continue;
            }
            for (month, totals) in months {
                let month = month.trim();
                if month.is_empty() || totals.is_zero() {
                    continue;
                }
                if self
                    .apps
                    .entry(app.clone())
                    .or_default()
                    .entry(month.to_owned())
                    .or_default()
                    .merge_max(totals)
                {
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Compteurs d'activité par appareil (`device_id` → instantané), échangés lors
/// de la synchronisation Wi-Fi. Permet la convergence exacte sur 2, 3 ou N
/// appareils sans jamais double-compter ni perdre de statistiques.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageBundle {
    pub devices: BTreeMap<String, UsageSnapshot>,
}

impl UsageBundle {
    pub fn is_empty(&self) -> bool {
        self.devices.values().all(UsageSnapshot::is_empty)
    }
}

/// Date UTC `AAAA-MM-JJ` à partir d'un horodatage Unix en millisecondes.
pub fn day_from_unix_ms(unix_ms: u64) -> String {
    let z = (unix_ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Totaux par jour (`AAAA-MM-JJ`), et par application et par mois, dans de
/// petits fichiers JSON. Les compteurs reçus d'autres appareils synchronisés
/// sont gardés séparément dans `usage-sync.json` (par identifiant d'appareil)
/// puis additionnés à la lecture.
pub struct Ledger {
    path: PathBuf,
    days: BTreeMap<String, Totals>,
    apps_path: PathBuf,
    apps: AppMonths,
    remote_path: PathBuf,
    remote: BTreeMap<String, UsageSnapshot>,
}

/// Fichier JSON ; absent ou illisible : valeur par défaut (réécrite au
/// prochain enregistrement).
fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Écriture atomique : une coupure ne corrompt pas les totaux.
fn write_json<T: Serialize>(path: &PathBuf, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
}

impl Ledger {
    /// Un fichier absent ou illisible repart de zéro (et sera réécrit).
    /// Les totaux par application sont à côté (`usage-apps.json`) ainsi que
    /// les compteurs reçus d'autres appareils synchronisés (`usage-sync.json`).
    pub fn open(path: PathBuf) -> Self {
        let apps_path = path.with_file_name("usage-apps.json");
        let remote_path = path.with_file_name("usage-sync.json");
        Self {
            days: read_json(&path),
            apps: read_json(&apps_path),
            remote: read_json(&remote_path),
            path,
            apps_path,
            remote_path,
        }
    }

    /// Recharge les compteurs synchronisés des autres appareils depuis
    /// `usage-sync.json` sans toucher aux compteurs locaux.
    pub fn reload_remote(&mut self) {
        self.remote = read_json(&self.remote_path);
    }

    /// `app` : application de destination, si elle est connue.
    pub fn record(
        &mut self,
        day: &str,
        app: Option<&str>,
        usage: &Usage,
        pricing: &Pricing,
    ) -> io::Result<()> {
        self.add(
            day,
            app,
            &Totals {
                requests: 1,
                input_tokens: usage.input_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                output_tokens: usage.output_tokens,
                cost: pricing.cost(usage),
                ..Totals::default()
            },
        )
    }

    pub fn record_dictation(
        &mut self,
        day: &str,
        app: Option<&str>,
        words: u64,
        audio_ms: u64,
        command: bool,
    ) -> io::Result<()> {
        self.add(
            day,
            app,
            &Totals {
                dictations: 1,
                commands: command.into(),
                words,
                audio_ms,
                ..Totals::default()
            },
        )
    }

    /// Exporte les statistiques connues (appareil local + appareils déjà
    /// synchronisés, pour la transitivité sur 3 appareils ou plus).
    pub fn export_bundle(&self, local_device_id: &str) -> UsageBundle {
        let mut devices = self.remote.clone();
        let local_id = local_device_id.trim();
        let local = UsageSnapshot {
            days: self.days.clone(),
            apps: self.apps.clone(),
        };
        if !local_id.is_empty() && !local.is_empty() {
            devices.insert(local_id.to_owned(), local);
        }
        UsageBundle { devices }
    }

    /// Fusionne les statistiques reçues d'un pair dans `usage-sync.json` en
    /// ignorant toujours `local_device_id` (l'appareil local fait autorité sur
    /// ses propres compteurs). Renvoie `true` si au moins un compteur a évolué.
    pub fn merge_bundle(
        &mut self,
        local_device_id: &str,
        bundle: &UsageBundle,
    ) -> io::Result<bool> {
        let local_id = local_device_id.trim();
        let mut changed = false;
        for (device_id, snapshot) in &bundle.devices {
            let dev = device_id.trim();
            if dev.is_empty() || dev == local_id || snapshot.is_empty() {
                continue;
            }
            if self
                .remote
                .entry(dev.to_owned())
                .or_default()
                .merge_max(snapshot)
            {
                changed = true;
            }
        }
        if changed {
            write_json(&self.remote_path, &self.remote)?;
        }
        Ok(changed)
    }

    /// Totaux d'un jour (`AAAA-MM-JJ`) sur tous les appareils, à zéro s'il n'y a rien eu.
    pub fn day(&self, day: &str) -> Totals {
        let mut totals = self.days.get(day).copied().unwrap_or_default();
        for snap in self.remote.values() {
            if let Some(other) = snap.days.get(day) {
                totals.add(other);
            }
        }
        totals
    }

    fn add(&mut self, day: &str, app: Option<&str>, totals: &Totals) -> io::Result<()> {
        self.days.entry(day.to_owned()).or_default().add(totals);
        write_json(&self.path, &self.days)?;
        let Some(app) = app.map(str::trim).filter(|a| !a.is_empty()) else {
            return Ok(());
        };
        let month = day.get(..7).unwrap_or(day);
        self.apps
            .entry(app.to_lowercase())
            .or_default()
            .entry(month.to_owned())
            .or_default()
            .add(totals);
        write_json(&self.apps_path, &self.apps)
    }

    /// Totaux par application (tous appareils confondus), du plus de mots au
    /// moins : pour le mois `AAAA-MM`, ou depuis le début (`None`). Les
    /// applications sans dictée sur la période sont omises.
    pub fn apps(&self, month: Option<&str>) -> Vec<AppTotals> {
        let mut combined: BTreeMap<String, Totals> = BTreeMap::new();
        let mut accumulate = |apps: &AppMonths| {
            for (app, months) in apps {
                for (m, t) in months {
                    if month.is_none_or(|w| m == w) {
                        combined.entry(app.clone()).or_default().add(t);
                    }
                }
            }
        };
        accumulate(&self.apps);
        for snap in self.remote.values() {
            accumulate(&snap.apps);
        }
        let mut found: Vec<AppTotals> = combined
            .into_iter()
            .filter(|(_, totals)| totals.dictations > 0)
            .map(|(app, totals)| AppTotals { app, totals })
            .collect();
        found.sort_by(|a, b| {
            b.totals
                .words
                .cmp(&a.totals.words)
                .then(b.totals.dictations.cmp(&a.totals.dictations))
                .then_with(|| a.app.cmp(&b.app))
        });
        found
    }

    /// `today` au format `AAAA-MM-JJ` ; le mois est son préfixe `AAAA-MM`.
    pub fn summary(&self, today: &str) -> Summary {
        let month = today.get(..7).unwrap_or(today);
        let mut summary = Summary::default();
        let mut accumulate = |days: &BTreeMap<String, Totals>| {
            for (day, totals) in days {
                summary.total.add(totals);
                if day.starts_with(month) {
                    summary.month.add(totals);
                }
                if day == today {
                    summary.today.add(totals);
                }
            }
        };
        accumulate(&self.days);
        for snap in self.remote.values() {
            accumulate(&snap.days);
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing() -> Pricing {
        Pricing {
            input_per_million: 1.0,
            cached_input_per_million: 0.1,
            output_per_million: 2.0,
            currency: "USD".into(),
        }
    }

    fn usage(input: u64, cached: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: output,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vtt-usage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("usage.json")
    }

    #[test]
    fn cost_bills_cached_tokens_at_cache_price() {
        let cost = pricing().cost(&usage(1_000_000, 400_000, 500_000));
        assert!((cost - (0.6 + 0.04 + 1.0)).abs() < 1e-9, "{cost}");
        // Un cache annoncé plus grand que l'entrée ne rend pas le coût négatif.
        assert!(pricing().cost(&usage(10, 50, 0)) >= 0.0);
    }

    #[test]
    fn summary_splits_today_month_and_total() {
        let path = temp_path("summary");
        let mut ledger = Ledger::open(path.clone());
        ledger
            .record("2026-10-31", None, &usage(100, 0, 10), &pricing())
            .unwrap();
        ledger
            .record("2026-11-02", None, &usage(200, 0, 20), &pricing())
            .unwrap();
        ledger
            .record("2026-11-03", None, &usage(300, 0, 30), &pricing())
            .unwrap();
        ledger
            .record("2026-11-03", None, &usage(300, 0, 30), &pricing())
            .unwrap();

        let summary = Ledger::open(path).summary("2026-11-03");
        assert_eq!(summary.today.requests, 2);
        assert_eq!(summary.today.input_tokens, 600);
        assert_eq!(summary.month.requests, 3);
        assert_eq!(summary.total.requests, 4);
        assert_eq!(summary.total.output_tokens, 90);
    }

    #[test]
    fn dictations_and_costs_share_daily_totals() {
        let path = temp_path("activity");
        let mut ledger = Ledger::open(path.clone());
        ledger
            .record_dictation("2026-11-03", None, 12, 4000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-03", None, 5, 2000, true)
            .unwrap();
        ledger
            .record("2026-11-03", None, &usage(100, 0, 10), &pricing())
            .unwrap();

        let day = Ledger::open(path).day("2026-11-03");
        assert_eq!((day.dictations, day.commands, day.words), (2, 1, 17));
        assert_eq!(day.audio_ms, 6000);
        assert_eq!(day.requests, 1);
        assert_eq!(
            Ledger::open(temp_path("empty")).day("2026-11-03"),
            Totals::default()
        );
    }

    #[test]
    fn apps_are_ranked_by_words_per_month_or_overall() {
        let path = temp_path("apps");
        let mut ledger = Ledger::open(path.clone());
        ledger
            .record_dictation("2026-10-30", Some("Slack.exe"), 50, 9000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-02", Some("code.exe"), 20, 4000, false)
            .unwrap();
        ledger
            .record_dictation("2026-11-03", Some("slack.exe"), 10, 2000, false)
            .unwrap();
        ledger
            .record(
                "2026-11-03",
                Some("slack.exe"),
                &usage(100, 0, 10),
                &pricing(),
            )
            .unwrap();
        // Application inconnue : comptée dans le total du jour seulement.
        ledger
            .record_dictation("2026-11-03", None, 7, 1000, false)
            .unwrap();

        let ledger = Ledger::open(path);
        let names = |apps: Vec<AppTotals>| -> Vec<(String, u64)> {
            apps.into_iter().map(|a| (a.app, a.totals.words)).collect()
        };
        assert_eq!(
            names(ledger.apps(Some("2026-11"))),
            [("code.exe".into(), 20), ("slack.exe".into(), 10)]
        );
        assert_eq!(
            names(ledger.apps(None)),
            [("slack.exe".into(), 60), ("code.exe".into(), 20)]
        );
        let slack = &ledger.apps(None)[0].totals;
        assert_eq!((slack.dictations, slack.requests), (2, 1));
        assert!(ledger.apps(Some("2026-09")).is_empty());
        assert_eq!(ledger.day("2026-11-03").words, 17);
    }

    #[test]
    fn older_file_without_activity_fields_still_loads() {
        let path = temp_path("v030");
        fs::write(
            &path,
            r#"{"2026-11-03":{"requests":2,"input_tokens":800,"cached_input_tokens":0,"output_tokens":20,"cost":0.0003}}"#,
        )
        .unwrap();
        let day = Ledger::open(path).day("2026-11-03");
        assert_eq!((day.requests, day.dictations), (2, 0));
    }

    #[test]
    fn unreadable_file_starts_from_zero() {
        let path = temp_path("corrupt");
        fs::write(&path, "{pas du json").unwrap();
        let mut ledger = Ledger::open(path.clone());
        assert_eq!(ledger.summary("2026-11-03"), Summary::default());
        ledger
            .record("2026-11-03", None, &usage(1, 0, 1), &pricing())
            .unwrap();
        assert_eq!(Ledger::open(path).summary("2026-11-03").total.requests, 1);
    }

    #[test]
    fn day_from_unix_ms_formats_utc_dates_accurately() {
        assert_eq!(day_from_unix_ms(0), "1970-01-01");
        // 2024-02-29 (année bissextile) = 19782 jours après 1970-01-01.
        assert_eq!(day_from_unix_ms(19_782 * 86_400_000), "2024-02-29");
        // 2026-11-20T00:00:00Z = 1_795_132_800_000 ms (20777 jours).
        assert_eq!(
            day_from_unix_ms(20_777 * 86_400_000 + 3_600_000),
            "2026-11-20"
        );
    }

    #[test]
    fn multi_device_sync_converges_across_three_devices_without_double_counting() {
        let pc_path = temp_path("sync-pc");
        let phone_path = temp_path("sync-phone");
        let deck_path = temp_path("sync-deck");

        let mut pc = Ledger::open(pc_path.clone());
        let mut phone = Ledger::open(phone_path.clone());
        let mut deck = Ledger::open(deck_path.clone());

        pc.record_dictation("2026-11-20", Some("Slack.exe"), 100, 30_000, false)
            .unwrap();
        pc.record(
            "2026-11-20",
            Some("Slack.exe"),
            &usage(1000, 0, 200),
            &pricing(),
        )
        .unwrap();
        phone
            .record_dictation("2026-11-20", Some("com.whatsapp"), 50, 15_000, false)
            .unwrap();
        phone
            .record_dictation("2026-11-19", Some("slack.exe"), 25, 8_000, true)
            .unwrap();
        deck.record_dictation("2026-11-20", Some("discord"), 30, 10_000, false)
            .unwrap();

        // 1. PC ↔ Téléphone
        let pc_b1 = pc.export_bundle("pc");
        let phone_b1 = phone.export_bundle("phone");
        assert!(pc.merge_bundle("pc", &phone_b1).unwrap());
        assert!(phone.merge_bundle("phone", &pc_b1).unwrap());
        assert_eq!(pc.summary("2026-11-20").total.words, 175);
        assert_eq!(phone.summary("2026-11-20").total.words, 175);

        // Resynchronisation immédiate : idempotente (aucun changement, aucun double comptage).
        assert!(
            !pc.merge_bundle("pc", &phone.export_bundle("phone"))
                .unwrap()
        );
        assert!(
            !phone
                .merge_bundle("phone", &pc.export_bundle("pc"))
                .unwrap()
        );
        assert_eq!(pc.summary("2026-11-20").total.words, 175);

        // 2. Téléphone ↔ Steam Deck (transitivité : le Deck récupère aussi le PC !)
        let phone_b2 = phone.export_bundle("phone");
        let deck_b1 = deck.export_bundle("deck");
        assert!(phone.merge_bundle("phone", &deck_b1).unwrap());
        assert!(deck.merge_bundle("deck", &phone_b2).unwrap());
        assert_eq!(deck.summary("2026-11-20").total.words, 205);
        assert_eq!(phone.summary("2026-11-20").total.words, 205);

        // 3. Nouvelle dictée sur le téléphone puis synchro PC ↔ Téléphone :
        //    le PC reçoit à la fois les 20 nouveaux mots du téléphone ET les 30 du Deck.
        phone
            .record_dictation("2026-11-20", Some("com.whatsapp"), 20, 6_000, false)
            .unwrap();
        let phone_b3 = phone.export_bundle("phone");
        assert!(pc.merge_bundle("pc", &phone_b3).unwrap());
        assert_eq!(pc.summary("2026-11-20").total.words, 225);
        assert_eq!(pc.summary("2026-11-20").today.words, 200);
        assert_eq!(pc.day("2026-11-19").words, 25);
        assert_eq!(pc.day("2026-11-20").words, 200);

        // Statistiques par application agrégées tous appareils confondus (slack.exe PC + téléphone).
        let apps = pc.apps(Some("2026-11"));
        assert_eq!(apps[0].app, "slack.exe");
        assert_eq!(apps[0].totals.words, 125);
        assert_eq!(apps[0].totals.dictations, 2);
        assert_eq!(apps[1].app, "com.whatsapp");
        assert_eq!(apps[1].totals.words, 70);

        // Rechargement à chaud (`reload_remote`) depuis un second handle en mémoire.
        let mut live_pc = Ledger::open(pc_path.clone());
        let deck_extra = {
            deck.record_dictation("2026-11-20", None, 15, 5_000, false)
                .unwrap();
            deck.export_bundle("deck")
        };
        assert!(pc.merge_bundle("pc", &deck_extra).unwrap());
        live_pc.reload_remote();
        assert_eq!(live_pc.summary("2026-11-20").total.words, 240);
    }
}
