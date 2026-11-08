//! Notes de version (`CHANGELOG.md` à la racine du dépôt) : affichées avec
//! chaque mise à jour proposée, puis consultables après l'installation.
//!
//! Format (vérifié par la CI, `.github/scripts/changelog.py check`) :
//!
//! ```text
//! ## 0.22.0 — 2026-11-08
//!
//! - Changement visible sur le PC et sur le téléphone.
//! - [PC] Changement propre à l'application Windows.
//! - [Android] Changement propre au téléphone.
//! ```
//!
//! Le même texte sert deux fois :
//! - embarqué dans l'application à la compilation ([`BUNDLED`]) : nouveautés
//!   des versions installées, lisibles hors ligne ;
//! - recopié par la CI dans le manifeste de mise à jour (champ `changelog`) :
//!   nouveautés d'une version pas encore installée.
//!
//! La lecture est tolérante (texte venu du réseau, ou écrit à la main) : une
//! ligne inattendue est ignorée ou rattachée au changement précédent, jamais
//! une erreur.

use serde::{Deserialize, Serialize};

/// Notes de toutes les versions publiées, embarquées dans l'application.
pub const BUNDLED: &str = include_str!("../../../CHANGELOG.md");

/// Au-delà, le texte (venu du manifeste) est coupé.
const MAX_LEN: usize = 64 * 1024;

const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];

/// Application qui affiche les notes : chacune ne montre que ses changements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Desktop,
    Android,
}

/// Version « X.Y.Z » comparable (ordre lexicographique du triplet).
type Version = (u64, u64, u64);

/// Nouveautés d'une version, prêtes à afficher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    pub version: String,
    /// Date de publication lisible (« 8 novembre 2026 »), si le titre en donne une.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    pub items: Vec<String>,
}

/// Section `## X.Y.Z` lue telle quelle (avant le tri par plateforme).
#[derive(Debug)]
struct Section {
    version: Version,
    date: Option<String>,
    /// Changements, avec la plateforme visée (`None` : les deux).
    items: Vec<(Option<Platform>, String)>,
}

/// « 0.22.0 », « v0.22.0 » ou « 0.22.0-test » (suffixe ignoré) → (0, 22, 0).
pub fn parse_version(text: &str) -> Option<Version> {
    let text = text.trim().trim_start_matches('v');
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Nouveautés destinées à `platform`, de la plus récente à la plus ancienne,
/// pour les versions après `after` (exclue) et jusqu'à `up_to` (incluse).
/// Une borne absente ou illisible ne limite rien. Les versions sans
/// changement pour cette plateforme sont omises.
pub fn releases(
    text: &str,
    platform: Platform,
    after: Option<&str>,
    up_to: Option<&str>,
) -> Vec<Release> {
    let after = after.and_then(parse_version);
    let up_to = up_to.and_then(parse_version);
    let mut sections: Vec<Section> = parse(text)
        .into_iter()
        .filter(|s| after.is_none_or(|a| s.version > a))
        .filter(|s| up_to.is_none_or(|u| s.version <= u))
        .collect();
    sections.sort_by_key(|s| std::cmp::Reverse(s.version));
    sections.dedup_by_key(|s| s.version);
    sections
        .into_iter()
        .filter_map(|section| {
            let items: Vec<String> = section
                .items
                .into_iter()
                .filter(|(target, _)| target.is_none_or(|t| t == platform))
                .map(|(_, text)| text)
                .collect();
            let (major, minor, patch) = section.version;
            (!items.is_empty()).then(|| Release {
                version: format!("{major}.{minor}.{patch}"),
                date: section.date,
                items,
            })
        })
        .collect()
}

/// Versions lancées sur cet appareil : nouveautés à montrer après une mise
/// à jour, jusqu'à ce que l'utilisateur les ait vues.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Launches {
    /// Dernière version lancée.
    pub last_run: String,
    /// Mise à jour dont les nouveautés n'ont pas encore été vues : version
    /// d'avant (vide si inconnue : installation antérieure à ce suivi).
    pub unseen_since: Option<String>,
}

impl Launches {
    /// Lancement de `current`. `existing_install` : l'application avait déjà
    /// servi (réglages présents), utile quand rien n'a encore été enregistré.
    /// Renvoie vrai juste après une mise à jour.
    pub fn launch(&mut self, current: &str, existing_install: bool) -> bool {
        let updated = if self.last_run.is_empty() {
            existing_install
        } else {
            matches!(
                (parse_version(&self.last_run), parse_version(current)),
                (Some(before), Some(now)) if before < now
            )
        };
        if updated && self.unseen_since.is_none() {
            // Deux mises à jour sans ouvrir l'accueil : les nouveautés des deux.
            self.unseen_since = Some(self.last_run.clone());
        }
        self.last_run = current.to_owned();
        updated
    }

    /// Nouveautés de la mise à jour pas encore vues (vide si aucune). Version
    /// d'avant inconnue : celles de la version installée seulement.
    pub fn news(&self, text: &str, platform: Platform) -> Vec<Release> {
        let Some(since) = self.unseen_since.as_deref() else {
            return Vec::new();
        };
        let mut news = releases(text, platform, Some(since), Some(&self.last_run));
        if parse_version(since).is_none() {
            news.truncate(1);
        }
        news
    }
}

fn parse(text: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    // Section en cours ; `None` avant le premier titre de version, ou sous un
    // titre qui n'en est pas un (ignoré).
    let mut current: Option<Section> = None;
    for line in truncate(text, MAX_LEN).lines() {
        if let Some(title) = line.strip_prefix("## ") {
            sections.extend(current.take());
            current = heading(title);
            continue;
        }
        let Some(section) = current.as_mut() else {
            continue;
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            Some(item) => {
                let (platform, text) = platform_tag(item.trim());
                section.items.push((platform, plain(text)));
            }
            // Suite du changement précédent (ligne repliée).
            None => {
                if let Some((_, text)) = section.items.last_mut() {
                    text.push(' ');
                    text.push_str(&plain(line));
                }
            }
        }
    }
    sections.extend(current);
    for section in &mut sections {
        section.items.retain(|(_, text)| !text.is_empty());
    }
    sections
}

/// « 0.22.0 — 2026-11-08 » (ou « [0.22.0] - 2026-11-08 ») → section vide.
fn heading(title: &str) -> Option<Section> {
    let first = title.split_whitespace().next()?;
    let version = parse_version(first.trim_matches(|c| c == '[' || c == ']'))?;
    Some(Section {
        version,
        date: date(title),
        items: Vec::new(),
    })
}

/// Première date « AAAA-MM-JJ » du titre, en toutes lettres.
fn date(title: &str) -> Option<String> {
    let bytes = title.as_bytes();
    (0..bytes.len().saturating_sub(9)).find_map(|start| {
        let candidate = title.get(start..start + 10)?;
        let mut parts = candidate.split('-');
        let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
        if year.len() != 4 || month.len() != 2 || day.len() != 2 {
            return None;
        }
        let (year, month, day) = (
            year.parse::<u32>().ok()?,
            month.parse::<usize>().ok()?,
            day.parse::<u32>().ok()?,
        );
        let name = MONTHS.get(month.checked_sub(1)?)?;
        (1..=31)
            .contains(&day)
            .then(|| format!("{day} {name} {year}"))
    })
}

/// « [PC] texte » → (Desktop, « texte ») ; préfixe inconnu laissé dans le texte.
fn platform_tag(item: &str) -> (Option<Platform>, &str) {
    let Some(rest) = item.strip_prefix('[') else {
        return (None, item);
    };
    let Some((tag, text)) = rest.split_once(']') else {
        return (None, item);
    };
    let platform = match tag.trim().to_lowercase().as_str() {
        "pc" | "windows" => Platform::Desktop,
        "android" => Platform::Android,
        _ => return (None, item),
    };
    (Some(platform), text.trim_start())
}

/// Texte affiché tel quel : marques Markdown courantes retirées.
fn plain(text: &str) -> String {
    text.replace("**", "").replace('`', "").trim().to_owned()
}

fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Nouveautés

Texte d'introduction, ignoré.
- même une liste avant la première version

## 0.22.0 — 2026-11-08

- Commun aux deux.
- [PC] Seulement sur le PC,
  avec une ligne repliée.
- [Android] Seulement sur le téléphone.

## [0.21.0] - 2026-11-07

- [android] Téléphone seul (casse ignorée).

## À venir

- Section sans numéro : ignorée.

## v0.20.0

* [Windows] Le **PC**, `code` retiré.
- [Linux] Préfixe inconnu : gardé, pour les deux.
";

    fn versions(releases: &[Release]) -> Vec<&str> {
        releases.iter().map(|r| r.version.as_str()).collect()
    }

    #[test]
    fn filters_by_platform() {
        let desktop = releases(SAMPLE, Platform::Desktop, None, None);
        assert_eq!(versions(&desktop), ["0.22.0", "0.20.0"]);
        assert_eq!(
            desktop[0].items,
            [
                "Commun aux deux.",
                "Seulement sur le PC, avec une ligne repliée."
            ]
        );
        assert_eq!(
            desktop[1].items,
            [
                "Le PC, code retiré.",
                "[Linux] Préfixe inconnu : gardé, pour les deux."
            ]
        );

        let android = releases(SAMPLE, Platform::Android, None, None);
        assert_eq!(versions(&android), ["0.22.0", "0.21.0", "0.20.0"]);
        assert_eq!(
            android[0].items,
            ["Commun aux deux.", "Seulement sur le téléphone."]
        );
        assert_eq!(android[1].items, ["Téléphone seul (casse ignorée)."]);
        assert_eq!(android[2].items.len(), 1);
    }

    #[test]
    fn dates_in_words() {
        let android = releases(SAMPLE, Platform::Android, None, None);
        assert_eq!(android[0].date.as_deref(), Some("8 novembre 2026"));
        assert_eq!(android[1].date.as_deref(), Some("7 novembre 2026"));
        assert_eq!(android[2].date, None);
        assert_eq!(date("0.1.0 — 2026-13-01"), None);
        assert_eq!(date("0.1.0 — 2026-02-01"), Some("1 février 2026".into()));
        // Pas de date en caractères multi-octets : pas de panique.
        assert_eq!(date("0.1.0 — éé"), None);
    }

    #[test]
    fn bounds() {
        let between = releases(SAMPLE, Platform::Android, Some("0.20.0"), Some("0.21.0"));
        assert_eq!(versions(&between), ["0.21.0"]);
        // Version de test (suffixe) et bornes illisibles.
        let test = releases(SAMPLE, Platform::Android, Some("0.21.0-test"), Some("?"));
        assert_eq!(versions(&test), ["0.22.0"]);
        assert!(releases(SAMPLE, Platform::Desktop, Some("0.22.0"), None).is_empty());
    }

    #[test]
    fn newest_first_without_duplicates() {
        let text = "## 0.1.0\n- a\n## 0.3.0\n- c\n## 0.2.0\n- b\n## 0.3.0\n- c bis\n";
        let all = releases(text, Platform::Desktop, None, None);
        assert_eq!(versions(&all), ["0.3.0", "0.2.0", "0.1.0"]);
    }

    #[test]
    fn tolerant() {
        assert!(releases("", Platform::Desktop, None, None).is_empty());
        assert!(releases("## 1.0.0\n\n", Platform::Desktop, None, None).is_empty());
        assert!(releases("## 1.0.0\n- \n- [PC]\n", Platform::Desktop, None, None).is_empty());
        // Texte trop long coupé sans casser un caractère.
        let long = format!("## 1.0.0\n- {}", "é".repeat(MAX_LEN));
        assert_eq!(releases(&long, Platform::Desktop, None, None).len(), 1);
    }

    #[test]
    fn versions_parse() {
        assert_eq!(parse_version("0.22.0"), Some((0, 22, 0)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.22.0-test"), Some((0, 22, 0)));
        assert_eq!(parse_version("0.22"), None);
        assert_eq!(parse_version("0.22.0.1"), None);
        assert_eq!(parse_version("à venir"), None);
    }

    #[test]
    fn launches() {
        // Première installation : rien à annoncer.
        let mut fresh = Launches::default();
        assert!(!fresh.launch("0.22.0", false));
        assert_eq!(fresh.unseen_since, None);
        assert!(!fresh.launch("0.22.0", true));

        // Mise à jour : nouveautés depuis la version d'avant, gardées jusqu'à
        // ce qu'elles soient vues, même après une seconde mise à jour.
        let mut seen = fresh.clone();
        assert!(seen.launch("0.23.0", true));
        assert_eq!(seen.unseen_since.as_deref(), Some("0.22.0"));
        assert!(seen.launch("0.24.0", true));
        assert_eq!(seen.unseen_since.as_deref(), Some("0.22.0"));
        assert!(!seen.launch("0.24.0", true));
        assert_eq!(seen.last_run, "0.24.0");

        // Retour à une version plus ancienne : rien de neuf.
        let mut older = fresh.clone();
        assert!(!older.launch("0.21.0", true));
        assert_eq!(older.unseen_since, None);

        // Mise à jour depuis une version d'avant ce suivi (rien d'enregistré).
        let mut legacy = Launches::default();
        assert!(legacy.launch("0.22.0", true));
        assert_eq!(legacy.unseen_since.as_deref(), Some(""));
    }

    #[test]
    fn news_after_update() {
        let mut launches = Launches {
            last_run: "0.20.0".into(),
            unseen_since: None,
        };
        assert!(launches.news(SAMPLE, Platform::Android).is_empty());
        launches.launch("0.22.0", true);
        let news = launches.news(SAMPLE, Platform::Android);
        assert_eq!(versions(&news), ["0.22.0", "0.21.0"]);
        // Version d'avant inconnue : seulement la version installée.
        launches.unseen_since = Some(String::new());
        assert_eq!(
            versions(&launches.news(SAMPLE, Platform::Android)),
            ["0.22.0"]
        );
        // Vu : plus rien.
        launches.unseen_since = None;
        assert!(launches.news(SAMPLE, Platform::Desktop).is_empty());
    }

    /// Le fichier livré : la version compilée a ses notes, et les sections
    /// vont de la plus récente à la plus ancienne.
    #[test]
    fn bundled_changelog() {
        let sections = parse(BUNDLED);
        assert!(!sections.is_empty());
        for pair in sections.windows(2) {
            assert!(
                pair[0].version > pair[1].version,
                "CHANGELOG.md : {:?} avant {:?}",
                pair[0].version,
                pair[1].version
            );
        }
        let current = env!("CARGO_PKG_VERSION");
        assert_eq!(sections[0].version, parse_version(current).unwrap());
        for section in &sections {
            assert!(
                !section.items.is_empty(),
                "{:?} sans changement",
                section.version
            );
            assert!(section.date.is_some(), "{:?} sans date", section.version);
        }
    }
}
