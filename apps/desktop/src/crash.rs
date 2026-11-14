//! Linux : préparation de la session et trace des plantages.
//!
//! Un plantage dans une bibliothèque C ou C++ (GTK, WebKit, pilote graphique,
//! whisper.cpp) arrête le programme sans passer par le journal Rust : seul
//! un message sur la sortie d'erreur en reste, invisible quand l'application
//! est lancée d'un double-clic. Cette sortie est donc recopiée dans
//! `logs/stderr.log` (celle du lancement précédent reste dans
//! `stderr.precedent.log`), et un signal fatal y laisse une ligne.

use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::Path;

const STDERR_FILE: &str = "stderr.log";
const PREVIOUS_STDERR_FILE: &str = "stderr.precedent.log";

/// Variables d'environnement à poser avant toute bibliothèque graphique.
/// Appelé en tout début de `main`, avant le moindre thread.
pub fn prepare_session() {
    // WebKitGTK plante au démarrage sur certains pilotes et sous Wayland
    // (« Could not create default EGL display », « Error 71 ») avec son
    // rendu DMA-BUF : contournement recommandé par Tauri. Sans effet visible
    // pour nos fenêtres simples. Une valeur déjà posée par l'utilisateur
    // est respectée.
    set_default("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    // Raccourci, collage et pastille passent par X11 : sous Wayland, nos
    // fenêtres passent donc aussi par XWayland (l'AppImage le fait déjà).
    if std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty()) {
        set_default("GDK_BACKEND", "x11");
    }
}

fn set_default(name: &str, value: &str) {
    if std::env::var_os(name).is_none() {
        // SAFETY: appelé au tout début de `main`, avant tout autre thread.
        unsafe { std::env::set_var(name, value) };
    }
}

/// Recopie la sortie d'erreur dans `logs_dir/stderr.log` et note les signaux
/// fatals.
pub fn capture_stderr(logs_dir: &Path) {
    let path = logs_dir.join(STDERR_FILE);
    if path.exists() {
        let previous = logs_dir.join(PREVIOUS_STDERR_FILE);
        let _ = fs::rename(&path, &previous);
        report_previous_crash(&previous);
    }
    let file = match File::create(&path) {
        Ok(file) => file,
        Err(e) => {
            log::warn!("sortie d'erreur non recopiée ({}) : {e}", path.display());
            return;
        }
    };
    // SAFETY: dup2 remplace le descripteur 2 par une copie de celui du
    // fichier ; `file` peut ensuite être fermé.
    if unsafe { libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) } < 0 {
        log::warn!(
            "sortie d'erreur non recopiée : {}",
            std::io::Error::last_os_error()
        );
        return;
    }
    install_signal_handlers();
    log::info!(
        "messages des bibliothèques et plantages : {}",
        path.display()
    );
}

/// Indices d'un arrêt brutal dans la sortie d'erreur.
const CRASH_MARKERS: [&str; 6] = [
    "VoiceToText : plantage",
    "terminate called",
    "Aborting",
    "panicked at",
    "Error 71",
    "Gdk-ERROR",
];
/// Lignes de la fin de la sortie recopiées dans le journal.
const CRASH_TAIL_LINES: usize = 15;

/// Le lancement précédent s'est arrêté brutalement : la fin de sa sortie
/// d'erreur est recopiée dans le journal principal, le seul fichier qu'on
/// demande d'habitude.
fn report_previous_crash(previous: &Path) {
    let Ok(content) = fs::read(previous) else {
        return;
    };
    let text = String::from_utf8_lossy(&content);
    if !CRASH_MARKERS.iter().any(|marker| text.contains(marker)) {
        return;
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(CRASH_TAIL_LINES)..];
    log::error!(
        "le lancement précédent s'est arrêté brutalement ; fin de {} :\n{}",
        previous.display(),
        tail.join("\n")
    );
}

const FATAL_SIGNALS: [(libc::c_int, &str); 5] = [
    (libc::SIGSEGV, "SIGSEGV (accès mémoire invalide)\n"),
    (
        libc::SIGABRT,
        "SIGABRT (arrêt demandé par une bibliothèque)\n",
    ),
    (libc::SIGBUS, "SIGBUS (erreur de bus)\n"),
    (libc::SIGILL, "SIGILL (instruction illégale)\n"),
    (libc::SIGFPE, "SIGFPE (erreur arithmétique)\n"),
];

fn install_signal_handlers() {
    for (signal, _) in FATAL_SIGNALS {
        // SAFETY: gestionnaire qui n'appelle que des fonctions sûres dans un
        // gestionnaire de signal (write, raise) ; SA_RESETHAND rétablit le
        // comportement par défaut, SA_ONSTACK utilise la pile de secours de
        // Rust (débordement de pile).
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_fatal_signal as *const () as libc::sighandler_t;
            action.sa_flags = libc::SA_RESETHAND | libc::SA_ONSTACK;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

extern "C" fn on_fatal_signal(signal: libc::c_int) {
    const PREFIX: &[u8] = b"\nVoiceToText : plantage, signal ";
    let name = FATAL_SIGNALS
        .iter()
        .find(|(s, _)| *s == signal)
        .map_or("inconnu\n", |(_, name)| name);
    // Une seule ligne, même si une autre bibliothèque (WebKit) qui a chaîné
    // son gestionnaire au nôtre nous rappelle.
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let first = !REPORTED.swap(true, std::sync::atomic::Ordering::SeqCst);
    // SAFETY: write, signal et raise sont sûrs dans un gestionnaire de
    // signal. Comportement par défaut forcé : le signal relancé arrête le
    // programme au lieu de repasser par un gestionnaire chaîné.
    unsafe {
        if first {
            libc::write(libc::STDERR_FILENO, PREFIX.as_ptr().cast(), PREFIX.len());
            libc::write(libc::STDERR_FILENO, name.as_ptr().cast(), name.len());
        }
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}
