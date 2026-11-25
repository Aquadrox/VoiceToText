//! Sauvegarde et restauration multi-formats du presse-papier sous Windows
//! (texte Unicode/ANSI, HTML, RTF, images PNG/DIB, fichiers Explorer CF_HDROP
//! et marqueurs de confidentialité de l'historique Windows).
//!
//! Seuls les formats dont la poignée Win32 est un tampon `HGLOBAL` autonome
//! sont copiés ; les poignées GDI (`CF_BITMAP`, `CF_PALETTE`, `CF_ENHMETAFILE`)
//! et les formats OLE privés ne sont jamais verrouillés par `GlobalLock`. Si
//! seule une image `CF_BITMAP` est présente, Windows synthétise `CF_DIB` à la
//! demande, ce qui préserve quand même l'image.

use std::ptr;
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

use crate::clipboard::{
    CapturedClipboard, ClipboardSnapshot, PendingRestore, is_safe_windows_registered_name,
    is_safe_windows_standard_format, should_synthesize_dib, win_cf, windows_capture_priority,
};

/// Nombre de tentatives pour ouvrir le presse-papier (une autre application ou
/// l'historique Windows peut le verrouiller quelques millisecondes).
const OPEN_RETRIES: usize = 10;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(10);

pub(crate) type WindowsSnapshot = ClipboardSnapshot<u32>;

static PENDING: PendingRestore<u32, u32> = PendingRestore::new();

/// Garde RAII : garantit `CloseClipboard` même en cas de retour anticipé.
struct ClipboardGuard;

impl ClipboardGuard {
    fn open() -> Option<Self> {
        for attempt in 0..OPEN_RETRIES {
            // SAFETY: ouverture du presse-papier sans fenêtre propriétaire ;
            // refermé systématiquement dans `Drop`.
            if unsafe { OpenClipboard(None) }.is_ok() {
                return Some(Self);
            }
            if attempt + 1 < OPEN_RETRIES {
                thread::sleep(OPEN_RETRY_DELAY);
            }
        }
        None
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: `ClipboardGuard` n'existe que lorsque `OpenClipboard` a réussi.
        let _ = unsafe { CloseClipboard() };
    }
}

/// Capture tous les formats sûrs actuellement présents dans le presse-papier.
pub(crate) fn capture() -> CapturedClipboard<u32> {
    let Some(_guard) = ClipboardGuard::open() else {
        log::warn!("presse-papier Windows occupé : sauvegarde ignorée");
        return CapturedClipboard::Unrestorable;
    };
    capture_locked()
}

/// Si une restauration différée est encore active et que le presse-papier
/// contient toujours notre texte précédent (numéro de séquence inchangé),
/// reprend l'instantané d'origine ; sinon capture le presse-papier actuel.
pub(crate) fn capture_for_insert() -> CapturedClipboard<u32> {
    if let Some(inherited) = PENDING
        .take_or_invalidate(|&expected_seq| unsafe { GetClipboardSequenceNumber() } == expected_seq)
    {
        return CapturedClipboard::Snapshot(inherited);
    }
    capture()
}

/// Annule toute restauration différée en attente (copie explicite).
pub(crate) fn invalidate_pending() {
    PENDING.invalidate();
}

/// Planifie la restauration de `snapshot` après `delay`, uniquement si aucun
/// autre programme (ni une dictée suivante) n'a modifié le presse-papier
/// entre-temps.
pub(crate) fn schedule_restore(snapshot: WindowsSnapshot, seq_after_set: u32, delay: Duration) {
    if snapshot.is_empty() {
        return;
    }
    let generation = PENDING.arm(snapshot, seq_after_set);
    thread::spawn(move || {
        thread::sleep(delay);
        let Some((snapshot, expected_seq)) = PENDING.take_if_current(generation) else {
            return;
        };
        let Some(_guard) = ClipboardGuard::open() else {
            return;
        };
        // Vérification atomique sous verrou du presse-papier : ne jamais
        // écraser ce que l'utilisateur aurait copié entre-temps.
        if unsafe { GetClipboardSequenceNumber() } != expected_seq {
            return;
        }
        if !write_snapshot_locked(&snapshot) {
            log::warn!("restauration complète du presse-papier Windows incomplète");
        }
    });
}

/// Restaure immédiatement l'état capturé (utilisé par `copy_selection`).
pub(crate) fn restore_captured(captured: CapturedClipboard<u32>) {
    PENDING.invalidate();
    match captured {
        CapturedClipboard::Snapshot(snapshot) if !snapshot.is_empty() => {
            if let Some(_guard) = ClipboardGuard::open() {
                let _ = write_snapshot_locked(&snapshot);
            }
        }
        CapturedClipboard::Empty => {
            if let Some(_guard) = ClipboardGuard::open() {
                // SAFETY: presse-papier ouvert par `_guard`.
                let _ = unsafe { EmptyClipboard() };
            }
        }
        CapturedClipboard::Snapshot(_) | CapturedClipboard::Unrestorable => {}
    }
}

fn capture_locked() -> CapturedClipboard<u32> {
    let mut saw_any = false;
    let mut saw_bitmap = false;
    let mut candidates: Vec<(u8, u32)> = Vec::new();
    let mut format = 0u32;

    loop {
        // SAFETY: presse-papier ouvert par l'appelant.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        saw_any = true;
        if format == win_cf::BITMAP {
            saw_bitmap = true;
            continue;
        }
        if format < win_cf::REGISTERED_FIRST {
            if is_safe_windows_standard_format(format) {
                candidates.push((windows_capture_priority(format, None), format));
            }
        } else if let Some(name) = format_name(format)
            && is_safe_windows_registered_name(&name)
        {
            candidates.push((windows_capture_priority(format, Some(&name)), format));
        }
    }

    if !saw_any {
        return CapturedClipboard::Empty;
    }

    candidates.sort_by_key(|&(priority, _)| priority);

    let mut snapshot = WindowsSnapshot::new();
    let mut saved_formats = Vec::with_capacity(candidates.len());
    for (_, fmt) in candidates {
        if let Some(bytes) = read_hglobal_locked(fmt, &snapshot)
            && snapshot.push(fmt, bytes)
        {
            saved_formats.push(fmt);
        }
    }

    // Repli image : si seule une poignée CF_BITMAP a été posée par l'application
    // d'origine, Windows synthétise CF_DIB à la lecture.
    if should_synthesize_dib(saw_bitmap, &saved_formats)
        && unsafe { IsClipboardFormatAvailable(win_cf::DIB) }.is_ok()
        && let Some(bytes) = read_hglobal_locked(win_cf::DIB, &snapshot)
    {
        snapshot.push(win_cf::DIB, bytes);
    }

    if snapshot.is_empty() {
        CapturedClipboard::Unrestorable
    } else {
        CapturedClipboard::Snapshot(snapshot)
    }
}

fn format_name(format: u32) -> Option<String> {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` est un tampon valide de 256 `u16`.
    let len = unsafe { GetClipboardFormatNameW(format, &mut buf) };
    if len <= 0 {
        return None;
    }
    let len = (len as usize).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

fn read_hglobal_locked(format: u32, snapshot: &WindowsSnapshot) -> Option<Vec<u8>> {
    // SAFETY: presse-papier ouvert ; `format` a été filtré comme format HGLOBAL.
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let hglobal = HGLOBAL(handle.0);
    // SAFETY: `hglobal` appartient au presse-papier (jamais libéré ici).
    let size = unsafe { GlobalSize(hglobal) };
    if size == 0 || !snapshot.can_fit(size) {
        return None;
    }
    // SAFETY: verrouillage temporaire du bloc HGLOBAL valide tant que le
    // presse-papier reste ouvert.
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` pointe sur au moins `size` octets valides.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) }.to_vec();
    // SAFETY: déverrouillage symétrique du même `hglobal`.
    let _ = unsafe { GlobalUnlock(hglobal) };
    Some(bytes)
}

fn write_snapshot_locked(snapshot: &WindowsSnapshot) -> bool {
    // SAFETY: presse-papier ouvert par l'appelant.
    if unsafe { EmptyClipboard() }.is_err() {
        return false;
    }
    let mut restored_any = false;
    for entry in snapshot.entries() {
        if write_hglobal_locked(entry.format, &entry.data) {
            restored_any = true;
        }
    }
    restored_any
}

fn write_hglobal_locked(format: u32, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    // SAFETY: allocation d'un bloc déplaçable de `bytes.len()` octets.
    let Ok(hglobal) = (unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }) else {
        return false;
    };
    // SAFETY: `hglobal` vient d'être alloué avec `GMEM_MOVEABLE`.
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        // SAFETY: libération en cas d'échec avant transfert au presse-papier.
        let _ = unsafe { GlobalFree(Some(hglobal)) };
        return false;
    }
    // SAFETY: `ptr` dispose de `bytes.len()` octets non recouvrants.
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(hglobal);
    }
    // SAFETY: si `SetClipboardData` réussit, Windows devient propriétaire de
    // `hglobal` (ne pas le libérer) ; en cas d'échec, nous le libérons.
    if unsafe { SetClipboardData(format, Some(HANDLE(hglobal.0))) }.is_err() {
        let _ = unsafe { GlobalFree(Some(hglobal)) };
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::core::PCWSTR;

    static CLIPBOARD_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn register_format(name: &str) -> u32 {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: chaîne UTF-16 terminée par un zéro.
        unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
    }

    #[test]
    fn round_trip_multiformat_html_png_and_text() {
        let _lock = CLIPBOARD_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let html_fmt = register_format("HTML Format");
        let png_fmt = register_format("PNG");
        assert_ne!(html_fmt, 0);
        assert_ne!(png_fmt, 0);

        let html_bytes = b"Version:0.9\r\n<b>Salut</b>\0".to_vec();
        let png_bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
        let text_utf16: Vec<u8> = "Salut"
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();

        let mut initial = WindowsSnapshot::new();
        assert!(initial.push(win_cf::UNICODETEXT, text_utf16.clone()));
        assert!(initial.push(html_fmt, html_bytes.clone()));
        assert!(initial.push(png_fmt, png_bytes.clone()));

        {
            let _guard = ClipboardGuard::open().expect("ouverture presse-papier");
            assert!(write_snapshot_locked(&initial));
        }

        let captured = capture();
        let snapshot = captured.into_snapshot().expect("instantané non vide");
        assert!(
            snapshot
                .entries()
                .iter()
                .any(|e| e.format == html_fmt && e.data.starts_with(&html_bytes))
        );
        assert!(
            snapshot
                .entries()
                .iter()
                .any(|e| e.format == png_fmt && e.data.starts_with(&png_bytes))
        );

        // Écrasement par un simple texte dicté, puis restauration différée.
        let mut dict = WindowsSnapshot::new();
        let dictated: Vec<u8> = "Dictée"
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        dict.push(win_cf::UNICODETEXT, dictated);
        {
            let _guard = ClipboardGuard::open().expect("ouverture presse-papier");
            assert!(write_snapshot_locked(&dict));
        }
        let seq = unsafe { GetClipboardSequenceNumber() };

        schedule_restore(snapshot, seq, Duration::from_millis(50));
        thread::sleep(Duration::from_millis(180));

        let restored = capture().into_snapshot().expect("restauré");
        assert!(
            restored
                .entries()
                .iter()
                .any(|e| e.format == html_fmt && e.data.starts_with(&html_bytes))
        );
        assert!(
            restored
                .entries()
                .iter()
                .any(|e| e.format == png_fmt && e.data.starts_with(&png_bytes))
        );
    }

    #[test]
    fn restore_skips_when_user_copied_something_new() {
        let _lock = CLIPBOARD_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let png_fmt = register_format("PNG");
        let mut orig = WindowsSnapshot::new();
        orig.push(png_fmt, vec![0x89, b'P', b'N', b'G', 9, 9]);

        let mut dictated = WindowsSnapshot::new();
        dictated.push(
            win_cf::UNICODETEXT,
            "temporaire"
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        );
        {
            let _guard = ClipboardGuard::open().expect("ouverture presse-papier");
            assert!(write_snapshot_locked(&dictated));
        }
        let seq = unsafe { GetClipboardSequenceNumber() };
        schedule_restore(orig, seq, Duration::from_millis(100));

        // L'utilisateur copie autre chose avant la fin du délai.
        let mut user_copy = WindowsSnapshot::new();
        let expected: Vec<u8> = "copie utilisateur"
            .encode_utf16()
            .chain(Some(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        user_copy.push(win_cf::UNICODETEXT, expected.clone());
        {
            let _guard = ClipboardGuard::open().expect("ouverture presse-papier");
            assert!(write_snapshot_locked(&user_copy));
        }

        thread::sleep(Duration::from_millis(200));
        let after = capture()
            .into_snapshot()
            .expect("toujours la copie utilisateur");
        assert!(!after.entries().iter().any(|e| e.format == png_fmt));
        assert!(
            after
                .entries()
                .iter()
                .any(|e| e.format == win_cf::UNICODETEXT && e.data.starts_with(&expected))
        );
    }
}
