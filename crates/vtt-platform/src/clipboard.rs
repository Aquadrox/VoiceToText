//! Logique commune de sauvegarde et restauration multi-formats du
//! presse-papier : filtrage des formats sûrs (texte, HTML, RTF, images,
//! fichiers, indicateurs de confidentialité), bornes mémoire et coordination
//! des restaurations différées lors d'insertions rapides successives.

use std::sync::Mutex;

/// Taille maximale conservée pour un seul format (16 Mio).
pub(crate) const MAX_FORMAT_BYTES: usize = 16 * 1024 * 1024;

/// Taille maximale cumulée d'un instantané complet du presse-papier (32 Mio).
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

/// Nombre maximal de formats conservés dans un instantané.
pub(crate) const MAX_FORMATS: usize = 32;

/// Constantes Win32 standard du presse-papier (valeurs fixes de l'ABI Windows,
/// définies ici pour être partagées et testées sous Linux également).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod win_cf {
    pub const TEXT: u32 = 1;
    pub const BITMAP: u32 = 2;
    pub const TIFF: u32 = 6;
    pub const OEMTEXT: u32 = 7;
    pub const DIB: u32 = 8;
    pub const UNICODETEXT: u32 = 13;
    pub const HDROP: u32 = 15;
    pub const LOCALE: u32 = 16;
    pub const DIBV5: u32 = 17;
    /// Seuil à partir duquel un format est enregistré par nom (`RegisterClipboardFormat`).
    pub const REGISTERED_FIRST: u32 = 0xC000;
}

/// Une entrée (un format et ses octets bruts) d'un instantané du presse-papier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardEntry<K> {
    pub format: K,
    pub data: Vec<u8>,
}

/// Instantané multi-formats du presse-papier, borné en mémoire et en nombre
/// d'entrées pour éviter tout emballement sur une image géante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardSnapshot<K> {
    entries: Vec<ClipboardEntry<K>>,
    total_bytes: usize,
}

impl<K> Default for ClipboardSnapshot<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            total_bytes: 0,
        }
    }
}

impl<K: PartialEq> ClipboardSnapshot<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute un format si les limites de nombre et de mémoire le permettent et
    /// qu'il n'est pas déjà présent. Renvoie `true` si l'entrée a été gardée.
    pub fn push(&mut self, format: K, data: Vec<u8>) -> bool {
        if self.entries.len() >= MAX_FORMATS {
            return false;
        }
        if data.len() > MAX_FORMAT_BYTES {
            return false;
        }
        if self.total_bytes.saturating_add(data.len()) > MAX_SNAPSHOT_BYTES {
            return false;
        }
        if self.entries.iter().any(|entry| entry.format == format) {
            return false;
        }
        self.total_bytes += data.len();
        self.entries.push(ClipboardEntry { format, data });
        true
    }

    /// Indique si `size` octets supplémentaires tiennent encore dans le budget.
    pub fn can_fit(&self, size: usize) -> bool {
        self.entries.len() < MAX_FORMATS
            && size <= MAX_FORMAT_BYTES
            && self.total_bytes.saturating_add(size) <= MAX_SNAPSHOT_BYTES
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn entries(&self) -> &[ClipboardEntry<K>] {
        &self.entries
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn into_entries(self) -> Vec<ClipboardEntry<K>> {
        self.entries
    }
}

/// État capturé du presse-papier avant une opération temporaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CapturedClipboard<K> {
    /// Presse-papier vide avant l'opération.
    Empty,
    /// Au moins un format sûr a été sauvegardé.
    Snapshot(ClipboardSnapshot<K>),
    /// Contenu présent mais aucun format sûr/sous la limite n'a pu être copié.
    Unrestorable,
}

impl<K: PartialEq> CapturedClipboard<K> {
    pub fn into_snapshot(self) -> Option<ClipboardSnapshot<K>> {
        match self {
            Self::Snapshot(snapshot) if !snapshot.is_empty() => Some(snapshot),
            _ => None,
        }
    }
}

/// Coordonne la restauration différée lors d'insertions successives rapides
/// (par exemple l'insertion au fil de la dictée) :
/// si une nouvelle dictée est collée avant la fin du délai de 600 ms et que le
/// presse-papier contient encore notre texte précédent, on réutilise
/// l'instantané d'origine au lieu de sauvegarder notre propre texte temporaire.
pub(crate) struct PendingRestore<K, M> {
    state: Mutex<PendingState<K, M>>,
}

struct PendingState<K, M> {
    generation: u64,
    snapshot: Option<ClipboardSnapshot<K>>,
    marker: Option<M>,
}

impl<K, M> PendingRestore<K, M> {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(PendingState {
                generation: 0,
                snapshot: None,
                marker: None,
            }),
        }
    }

    /// Récupère l'instantané d'origine encore en attente si `still_ours` valide
    /// le marqueur de la dernière insertion ; invalide la génération précédente
    /// pour éviter qu'un ancien minuteur n'écrase la nouvelle insertion.
    pub fn take_or_invalidate(
        &self,
        still_ours: impl FnOnce(&M) -> bool,
    ) -> Option<ClipboardSnapshot<K>> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.generation = guard.generation.wrapping_add(1);
        let marker = guard.marker.take();
        let snapshot = guard.snapshot.take();
        match (marker, snapshot) {
            (Some(marker), Some(snapshot)) if still_ours(&marker) => Some(snapshot),
            _ => None,
        }
    }

    /// Enregistre une restauration en attente et renvoie son numéro de
    /// génération (à passer ensuite à [`Self::take_if_current`]).
    pub fn arm(&self, snapshot: ClipboardSnapshot<K>, marker: M) -> u64 {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.generation = guard.generation.wrapping_add(1);
        guard.snapshot = Some(snapshot);
        guard.marker = Some(marker);
        guard.generation
    }

    /// Annule toute restauration en attente (ex. copie explicite par
    /// l'utilisateur via `copy_text` ou `paste_image`).
    pub fn invalidate(&self) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.generation = guard.generation.wrapping_add(1);
        guard.snapshot = None;
        guard.marker = None;
    }

    /// Récupère l'instantané et son marqueur si et seulement si `generation` est
    /// toujours la dernière enregistrée.
    pub fn take_if_current(&self, generation: u64) -> Option<(ClipboardSnapshot<K>, M)> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if guard.generation != generation {
            return None;
        }
        let snapshot = guard.snapshot.take()?;
        let marker = guard.marker.take()?;
        Some((snapshot, marker))
    }
}

/// Indique si un format standard Win32 (`< 0xC000`) est un tampon `HGLOBAL`
/// sûr à sauvegarder et restaurer par copie mémoire.
///
/// Les poignées GDI (`CF_BITMAP`, `CF_PALETTE`, `CF_ENHMETAFILE`,
/// `CF_METAFILEPICT`), les formats d'affichage (`CF_OWNERDISPLAY`, `CF_DSP*`)
/// et les plages privées/GDI sont strictement exclus : appeler `GlobalLock`
/// dessus serait invalide. Windows synthétise automatiquement `CF_BITMAP` à
/// partir de `CF_DIB` / `CF_DIBV5`.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_safe_windows_standard_format(format: u32) -> bool {
    matches!(
        format,
        win_cf::UNICODETEXT
            | win_cf::TEXT
            | win_cf::OEMTEXT
            | win_cf::LOCALE
            | win_cf::DIBV5
            | win_cf::DIB
            | win_cf::TIFF
            | win_cf::HDROP
    )
}

/// Formats enregistrés Win32 (`>= 0xC000`) dont les données sont des tampons
/// `HGLOBAL` autonomes sûrs (HTML, RTF, images, fichiers Explorer, marqueurs
/// de confidentialité de l'historique Windows).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn is_safe_windows_registered_name(name: &str) -> bool {
    const ALLOWED: &[&str] = &[
        // Texte riche, HTML, tableaux et liens.
        "html format",
        "rich text format",
        "rich text format without objects",
        "text/html",
        "text/rtf",
        "text/plain",
        "text/markdown",
        "text/csv",
        "csv",
        "uniformresourcelocator",
        "uniformresourcelocatorw",
        // Images (PNG conserve la transparence et prend bien moins de place que DIB).
        "png",
        "gif",
        "jfif",
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/bmp",
        "image/webp",
        "image/svg+xml",
        "image/tiff",
        // Fichiers copiés/coupés depuis l'Explorateur Windows.
        "preferred dropeffect",
        "filename",
        "filenamew",
        "shell idlist array",
        "text/uri-list",
        // Indicateurs de confidentialité (gestionnaires de mots de passe / Win+V).
        "excludeclipboardcontentfrommonitorprocessing",
        "canincludeinclipboardhistory",
        "canuploadtocloudclipboard",
    ];
    let lower = name.trim().to_ascii_lowercase();
    ALLOWED.contains(&lower.as_str())
}

/// Priorité de capture d'un format Windows (plus petit = capturé en premier) :
/// fichiers, texte, HTML/RTF et images compressées passent avant les gros DIB
/// non compressés afin de ne jamais les évincer si le plafond mémoire est atteint.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn windows_capture_priority(format: u32, name: Option<&str>) -> u8 {
    if let Some(name) = name {
        let lower = name.trim().to_ascii_lowercase();
        return match lower.as_str() {
            "excludeclipboardcontentfrommonitorprocessing"
            | "canincludeinclipboardhistory"
            | "canuploadtocloudclipboard"
            | "preferred dropeffect"
            | "filenamew"
            | "filename"
            | "shell idlist array"
            | "text/uri-list" => 0,
            "html format"
            | "rich text format"
            | "rich text format without objects"
            | "text/html"
            | "text/rtf"
            | "text/plain"
            | "text/markdown"
            | "text/csv"
            | "csv"
            | "uniformresourcelocatorw"
            | "uniformresourcelocator" => 1,
            "png" | "image/png" | "image/webp" | "image/jpeg" | "jfif" | "gif" | "image/gif"
            | "image/svg+xml" => 2,
            _ => 3,
        };
    }
    match format {
        win_cf::HDROP => 0,
        win_cf::UNICODETEXT | win_cf::LOCALE | win_cf::TEXT | win_cf::OEMTEXT => 1,
        win_cf::DIBV5 | win_cf::DIB | win_cf::TIFF => 3,
        _ => 4,
    }
}

/// Si l'application n'a posé qu'un `CF_BITMAP` sans `CF_DIB` ni `CF_DIBV5`,
/// Windows peut synthétiser `CF_DIB` à la lecture.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn should_synthesize_dib(saw_bitmap: bool, saved_formats: &[u32]) -> bool {
    saw_bitmap && !saved_formats.contains(&win_cf::DIB) && !saved_formats.contains(&win_cf::DIBV5)
}

/// Indique si une cible MIME / atome X11 Linux est un format de données sûr à
/// sauvegarder et restaurer (texte, HTML, RTF, images, fichiers, indicateurs de
/// confidentialité KDE). Les cibles de protocole X11 (`TARGETS`, `MULTIPLE`,
/// `TIMESTAMP`, `SAVE_TARGETS`, `DELETE`…) sont toujours écartées.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn is_safe_linux_target(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return false;
    }
    // Cibles de protocole ICCCM / X11 : jamais des données utilisateur.
    if matches!(
        trimmed,
        "TARGETS"
            | "MULTIPLE"
            | "TIMESTAMP"
            | "SAVE_TARGETS"
            | "DELETE"
            | "INSERT_SELECTION"
            | "INSERT_PROPERTY"
            | "ATOM_PAIR"
            | "text/x-moz-url-priv"
    ) || trimmed.starts_with("SAVE_")
    {
        return false;
    }
    // Atomes texte ICCCM classiques.
    if matches!(trimmed, "UTF8_STRING" | "STRING" | "TEXT" | "COMPOUND_TEXT") {
        return true;
    }
    // Type MIME de base (avant un éventuel `;charset=…`).
    let base = trimmed
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if base.starts_with("text/") || base.starts_with("image/") {
        return true;
    }
    matches!(
        base.as_str(),
        "application/rtf"
            | "application/xhtml+xml"
            | "application/x-kde-cutselection"
            | "x-special/gnome-copied-files"
            | "x-special/nautilus-clipboard"
            | "x-kde-passwordmanagerhint"
    )
}

/// Priorité de capture d'une cible Linux (plus petit = capturé en premier) :
/// fichiers, indicateurs de confidentialité, texte/HTML/RTF et images
/// compressées avant les gros formats bitmap non compressés (`image/bmp`…).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn linux_capture_priority(name: &str) -> u8 {
    let base = name
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match base.as_str() {
        "x-kde-passwordmanagerhint"
        | "text/uri-list"
        | "x-special/gnome-copied-files"
        | "x-special/nautilus-clipboard"
        | "application/x-kde-cutselection" => 0,
        "utf8_string"
        | "text/plain"
        | "text/html"
        | "text/rtf"
        | "application/rtf"
        | "application/xhtml+xml"
        | "text/markdown"
        | "text/csv"
        | "string"
        | "text"
        | "compound_text" => 1,
        "image/png" | "image/webp" | "image/jpeg" | "image/gif" | "image/svg+xml" => 2,
        s if s.starts_with("text/") => 1,
        s if s.starts_with("image/") => 3,
        _ => 4,
    }
}

/// Trie les cibles Linux par ordre de priorité de capture (stable, dédupliqué).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn sort_linux_targets(targets: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    targets.retain(|t| is_safe_linux_target(t) && seen.insert(t.clone()));
    targets.sort_by_key(|t| linux_capture_priority(t));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_enforces_limits_and_deduplicates() {
        let mut snap = ClipboardSnapshot::<&str>::new();
        assert!(snap.is_empty());
        assert!(snap.push("text/plain", b"bonjour".to_vec()));
        // Doublon ignoré.
        assert!(!snap.push("text/plain", b"autre".to_vec()));
        assert_eq!(snap.len(), 1);
        assert_eq!(snap.total_bytes(), 7);

        // Entrée dépassant la taille unitaire refusée.
        assert!(!snap.can_fit(MAX_FORMAT_BYTES + 1));
        assert!(!snap.push("image/bmp", vec![0u8; MAX_FORMAT_BYTES + 1]));
        assert_eq!(snap.len(), 1);

        // Deux entrées de 16 Mio remplissent exactement 32 Mio : la seconde ne
        // tient plus parce qu'il y a déjà 7 octets.
        assert!(snap.push("image/png", vec![1u8; MAX_FORMAT_BYTES]));
        assert!(!snap.can_fit(MAX_FORMAT_BYTES));
        assert!(!snap.push("image/tiff", vec![2u8; MAX_FORMAT_BYTES]));
        // Une petite entrée HTML tient toujours.
        assert!(snap.push("text/html", b"<b>ok</b>".to_vec()));
        assert_eq!(snap.len(), 3);
    }

    #[test]
    fn windows_format_filters_reject_gdi_and_ole() {
        assert!(is_safe_windows_standard_format(win_cf::UNICODETEXT));
        assert!(is_safe_windows_standard_format(win_cf::DIBV5));
        assert!(is_safe_windows_standard_format(win_cf::DIB));
        assert!(is_safe_windows_standard_format(win_cf::HDROP));
        // Poignées GDI / métafichiers / privés : jamais copiés comme HGLOBAL.
        assert!(!is_safe_windows_standard_format(win_cf::BITMAP));
        assert!(!is_safe_windows_standard_format(3)); // CF_METAFILEPICT
        assert!(!is_safe_windows_standard_format(9)); // CF_PALETTE
        assert!(!is_safe_windows_standard_format(14)); // CF_ENHMETAFILE
        assert!(!is_safe_windows_standard_format(0x0080)); // CF_OWNERDISPLAY
        assert!(!is_safe_windows_standard_format(0x0200)); // CF_PRIVATEFIRST
        assert!(!is_safe_windows_standard_format(0x0300)); // CF_GDIOBJFIRST

        // Formats enregistrés autorisés (insensibles à la casse).
        assert!(is_safe_windows_registered_name("HTML Format"));
        assert!(is_safe_windows_registered_name("Rich Text Format"));
        assert!(is_safe_windows_registered_name("PNG"));
        assert!(is_safe_windows_registered_name("Preferred DropEffect"));
        assert!(is_safe_windows_registered_name(
            "ExcludeClipboardContentFromMonitorProcessing"
        ));
        // Formats OLE / propriétaires dangereux : refusés.
        assert!(!is_safe_windows_registered_name("Ole Private Data"));
        assert!(!is_safe_windows_registered_name("Object Descriptor"));
        assert!(!is_safe_windows_registered_name("Embedded Object"));
    }

    #[test]
    fn windows_priority_and_dib_synthesis() {
        assert!(
            windows_capture_priority(win_cf::HDROP, None)
                < windows_capture_priority(win_cf::UNICODETEXT, None)
        );
        assert!(
            windows_capture_priority(0xC100, Some("HTML Format"))
                < windows_capture_priority(0xC101, Some("PNG"))
        );
        assert!(
            windows_capture_priority(0xC101, Some("PNG"))
                < windows_capture_priority(win_cf::DIB, None)
        );
        assert!(should_synthesize_dib(true, &[win_cf::UNICODETEXT]));
        assert!(!should_synthesize_dib(true, &[win_cf::DIB]));
        assert!(!should_synthesize_dib(true, &[win_cf::DIBV5]));
        assert!(!should_synthesize_dib(false, &[]));
    }

    #[test]
    fn linux_target_filtering_and_priority() {
        let mut targets = vec![
            "TARGETS".into(),
            "TIMESTAMP".into(),
            "SAVE_TARGETS".into(),
            "DELETE".into(),
            "MULTIPLE".into(),
            "text/x-moz-url-priv".into(),
            "image/bmp".into(),
            "image/png".into(),
            "text/html".into(),
            "UTF8_STRING".into(),
            "text/uri-list".into(),
            "x-special/gnome-copied-files".into(),
            "application/x-kde-cutselection".into(),
            "x-kde-passwordManagerHint".into(),
            "application/octet-stream".into(),
            "image/png".into(),
        ];
        sort_linux_targets(&mut targets);
        assert!(!targets.iter().any(|t| t == "TARGETS"));
        assert!(!targets.iter().any(|t| t == "SAVE_TARGETS"));
        assert!(!targets.iter().any(|t| t == "DELETE"));
        assert!(!targets.iter().any(|t| t == "text/x-moz-url-priv"));
        assert!(!targets.iter().any(|t| t == "application/octet-stream"));
        // PNG est gardé une seule fois, avant image/bmp, et après texte/fichiers.
        assert_eq!(targets.iter().filter(|t| *t == "image/png").count(), 1);
        let pos = |name: &str| targets.iter().position(|t| t == name).unwrap();
        assert!(pos("text/uri-list") < pos("text/html"));
        assert!(pos("text/html") < pos("image/png"));
        assert!(pos("image/png") < pos("image/bmp"));
    }

    #[test]
    fn pending_restore_preserves_original_across_rapid_inserts() {
        let pending = PendingRestore::<&str, u32>::new();
        let mut image = ClipboardSnapshot::new();
        image.push("image/png", vec![0x89, b'P', b'N', b'G']);

        // 1re insertion : arme la restauration avec l'image d'origine.
        let gen1 = pending.arm(image.clone(), 42);
        // 2e insertion rapide (avant 600 ms) : le presse-papier porte toujours
        // le marqueur 42 → on hérite de l'image d'origine et gen1 est invalidée.
        let inherited = pending.take_or_invalidate(|&m| m == 42);
        assert_eq!(inherited, Some(image.clone()));
        let gen2 = pending.arm(inherited.unwrap(), 43);

        // Le minuteur de gen1 se réveille : ignoré.
        assert!(pending.take_if_current(gen1).is_none());
        // Le minuteur de gen2 se réveille : récupère bien l'image d'origine.
        assert_eq!(pending.take_if_current(gen2), Some((image, 43)));
        // Déjà consommée.
        assert!(pending.take_if_current(gen2).is_none());
    }

    #[test]
    fn pending_restore_drops_original_if_user_copied_something_else() {
        let pending = PendingRestore::<&str, u32>::new();
        let mut old = ClipboardSnapshot::new();
        old.push("text/html", b"<i>old</i>".to_vec());
        let gen1 = pending.arm(old, 10);

        // L'utilisateur a copié autre chose (marqueur ≠ 10).
        let inherited = pending.take_or_invalidate(|&m| m == 99);
        assert!(inherited.is_none());
        // Le minuteur de gen1 est également invalidé.
        assert!(pending.take_if_current(gen1).is_none());
    }
}
