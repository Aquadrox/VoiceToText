//! Sauvegarde et restauration multi-formats du presse-papier sous Linux
//! (texte, HTML, RTF, images PNG/JPEG/BMP, fichiers `text/uri-list` et
//! indicateurs de confidentialité KDE), sur session Wayland (`wl-clipboard-rs`,
//! protocole `data-control`) comme sur session X11 (`x11rb`, sélection
//! `CLIPBOARD` avec prise en charge de `TARGETS`, `MULTIPLE` et `INCR` pour
//! les images volumineuses).

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    self, Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux,
    EventMask, PropMode, Property, SelectionNotifyEvent, SelectionRequestEvent, Window,
    WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::clipboard::{
    CapturedClipboard, ClipboardSnapshot, MAX_FORMAT_BYTES, PendingRestore, is_safe_linux_target,
    linux_capture_priority, sort_linux_targets,
};
use crate::linux::wayland_session;

/// Délai maximal pour lire une cible du presse-papier.
const TARGET_READ_TIMEOUT: Duration = Duration::from_millis(180);

/// Budget de temps total pour la sauvegarde complète avant une dictée.
const CAPTURE_TOTAL_TIMEOUT: Duration = Duration::from_millis(600);

/// Seuil au-delà duquel un transfert X11 passe en mode incrémental (`INCR`).
const X11_INCR_CHUNK_BYTES: usize = 64 * 1024;

/// Durée de vie maximale d'un transfert `INCR` en cours avant abandon.
const X11_INCR_TTL: Duration = Duration::from_secs(10);

pub(crate) type LinuxSnapshot = ClipboardSnapshot<String>;

#[derive(Debug, Clone)]
struct LinuxMarker {
    inserted_text: String,
    x11_owner: Option<Window>,
}

static PENDING: PendingRestore<String, LinuxMarker> = PendingRestore::new();

/// Sauvegarde tous les formats sûrs du presse-papier (Wayland puis repli X11).
pub(crate) fn capture() -> CapturedClipboard<String> {
    if wayland_session()
        && let Ok(captured) = capture_wayland()
    {
        return captured;
    }
    capture_x11()
}

/// Si une restauration différée est encore en attente et que le presse-papier
/// contient toujours notre texte précédent, reprend l'instantané d'origine ;
/// sinon capture le presse-papier courant.
pub(crate) fn capture_for_insert(
    read_text: impl FnOnce() -> Option<String>,
) -> CapturedClipboard<String> {
    let mut read_text = Some(read_text);
    if let Some(inherited) = PENDING.take_or_invalidate(|marker| {
        if let Some(expected_owner) = marker.x11_owner
            && !wayland_session()
            && current_x11_owner() != Some(expected_owner)
        {
            return false;
        }
        let current = read_text.take().and_then(|f| f());
        current.as_deref() == Some(marker.inserted_text.as_str())
    }) {
        return CapturedClipboard::Snapshot(inherited);
    }
    capture()
}

/// Annule toute restauration différée en attente (ex. copie explicite).
pub(crate) fn invalidate_pending() {
    PENDING.invalidate();
}

/// Enregistre l'état courant après écriture du texte dicté et planifie la
/// restauration de `snapshot` après `delay` si rien n'a changé entre-temps.
pub(crate) fn schedule_restore(
    snapshot: LinuxSnapshot,
    inserted_text: String,
    delay: Duration,
    read_text: fn() -> Option<String>,
) {
    if snapshot.is_empty() {
        return;
    }
    let x11_owner = if wayland_session() {
        None
    } else {
        current_x11_owner()
    };
    let marker = LinuxMarker {
        inserted_text,
        x11_owner,
    };
    let generation = PENDING.arm(snapshot, marker);
    thread::spawn(move || {
        thread::sleep(delay);
        let Some((snapshot, marker)) = PENDING.take_if_current(generation) else {
            return;
        };
        if let Some(expected_owner) = marker.x11_owner
            && !wayland_session()
            && current_x11_owner() != Some(expected_owner)
        {
            return;
        }
        if read_text().as_deref() != Some(marker.inserted_text.as_str()) {
            return;
        }
        if let Err(e) = restore_snapshot(snapshot) {
            log::warn!("restauration complète du presse-papier impossible : {e}");
        }
    });
}

/// Restaure immédiatement un état capturé (utilisé par `copy_selection`).
pub(crate) fn restore_captured(captured: CapturedClipboard<String>, clear: impl FnOnce()) {
    PENDING.invalidate();
    match captured {
        CapturedClipboard::Snapshot(snapshot) if !snapshot.is_empty() => {
            if let Err(e) = restore_snapshot(snapshot) {
                log::warn!("restauration du presse-papier impossible : {e}");
            }
        }
        CapturedClipboard::Empty => clear(),
        CapturedClipboard::Snapshot(_) | CapturedClipboard::Unrestorable => {}
    }
}

pub(crate) fn restore_snapshot(snapshot: LinuxSnapshot) -> Result<(), String> {
    if snapshot.is_empty() {
        return Ok(());
    }
    if wayland_session() {
        match restore_wayland(snapshot.clone()) {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::debug!("restauration Wayland indisponible ({e}), repli X11");
            }
        }
    }
    restore_x11(snapshot)
}

// ---------------------------------------------------------------------------
// Wayland (wl-clipboard-rs : protocole wlr/ext-data-control)
// ---------------------------------------------------------------------------

fn capture_wayland() -> Result<CapturedClipboard<String>, ()> {
    use wl_clipboard_rs::paste::{
        ClipboardType, Error as PasteError, MimeType, Seat, get_contents, get_mime_types_ordered,
    };

    let mut mimes = match get_mime_types_ordered(ClipboardType::Regular, Seat::Unspecified) {
        Ok(list) => list,
        Err(PasteError::ClipboardEmpty | PasteError::NoSeats) => {
            return Ok(CapturedClipboard::Empty);
        }
        Err(_) => return Err(()),
    };
    if mimes.is_empty() {
        return Ok(CapturedClipboard::Empty);
    }

    sort_linux_targets(&mut mimes);
    let total_deadline = Instant::now() + CAPTURE_TOTAL_TIMEOUT;
    let mut snapshot = LinuxSnapshot::new();

    for mime in mimes {
        let now = Instant::now();
        if now >= total_deadline {
            break;
        }
        let Ok((pipe, _)) = get_contents(
            ClipboardType::Regular,
            Seat::Unspecified,
            MimeType::Specific(&mime),
        ) else {
            continue;
        };
        let remaining = total_deadline.saturating_duration_since(now);
        let timeout = remaining.min(TARGET_READ_TIMEOUT);
        if let Some(bytes) = read_pipe_bounded(pipe, timeout, MAX_FORMAT_BYTES)
            && snapshot.can_fit(bytes.len())
        {
            snapshot.push(mime, bytes);
        }
    }

    if snapshot.is_empty() {
        Ok(CapturedClipboard::Unrestorable)
    } else {
        Ok(CapturedClipboard::Snapshot(snapshot))
    }
}

/// Lit un descripteur de tube en bornant à la fois le délai et la taille
/// (évite de bloquer la dictée sur une application figée ou une image géante).
fn read_pipe_bounded<R: Read + AsRawFd>(
    mut reader: R,
    timeout: Duration,
    max_bytes: usize,
) -> Option<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let fd = reader.as_raw_fd();
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];

    loop {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let wait_ms = deadline
            .saturating_duration_since(now)
            .as_millis()
            .min(i32::MAX as u128) as i32;
        if !poll_fd_readable(fd, wait_ms) {
            return None;
        }
        match reader.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                if out.len().saturating_add(n) > max_bytes {
                    return None;
                }
                out.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        }
    }
}

fn restore_wayland(snapshot: LinuxSnapshot) -> Result<(), String> {
    use wl_clipboard_rs::copy::{ClipboardType, MimeSource, MimeType, Options, Seat, Source};

    let sources: Vec<MimeSource> = snapshot
        .into_entries()
        .into_iter()
        .map(|entry| MimeSource {
            source: Source::Bytes(entry.data.into_boxed_slice()),
            mime_type: MimeType::Specific(entry.format),
        })
        .collect();
    if sources.is_empty() {
        return Ok(());
    }

    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    thread::Builder::new()
        .name("vtt-clip-wl".into())
        .spawn(move || {
            let mut opts = Options::new();
            opts.foreground(true)
                .omit_additional_text_mime_types(true)
                .clipboard(ClipboardType::Regular)
                .seat(Seat::All);
            match opts.prepare_copy_multi(sources) {
                Ok(prepared) => {
                    let _ = tx.send(Ok(()));
                    // Reste actif jusqu'à ce qu'une autre application prenne le
                    // presse-papier, puis quitte et libère la mémoire.
                    let _ = prepared.serve();
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("presse-papier Wayland : {e}")));
                }
            }
        })
        .map_err(|e| format!("thread presse-papier Wayland : {e}"))?;

    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "délai dépassé pour la restauration Wayland".to_owned())?
}

// ---------------------------------------------------------------------------
// X11 (x11rb : sélection CLIPBOARD, TARGETS, MULTIPLE et INCR)
// ---------------------------------------------------------------------------

struct X11Atoms {
    clipboard: Atom,
    targets: Atom,
    timestamp: Atom,
    multiple: Atom,
    save_targets: Atom,
    incr: Atom,
    atom_pair: Atom,
    transfer: Atom,
}

impl X11Atoms {
    fn intern(conn: &RustConnection) -> Option<Self> {
        let intern = |name: &[u8]| -> Option<Atom> {
            Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
        };
        Some(Self {
            clipboard: intern(b"CLIPBOARD")?,
            targets: intern(b"TARGETS")?,
            timestamp: intern(b"TIMESTAMP")?,
            multiple: intern(b"MULTIPLE")?,
            save_targets: intern(b"SAVE_TARGETS")?,
            incr: intern(b"INCR")?,
            atom_pair: intern(b"ATOM_PAIR")?,
            transfer: intern(b"VTT_CLIP_TRANSFER")?,
        })
    }
}

fn current_x11_owner() -> Option<Window> {
    let (conn, _) = x11rb::connect(None).ok()?;
    let clipboard = conn
        .intern_atom(false, b"CLIPBOARD")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let owner = conn
        .get_selection_owner(clipboard)
        .ok()?
        .reply()
        .ok()?
        .owner;
    (owner != x11rb::NONE).then_some(owner)
}

fn capture_x11() -> CapturedClipboard<String> {
    // Si notre propre thread de restauration possède déjà CLIPBOARD, relire
    // directement en mémoire évite tout aller-retour X11.
    if let Some(snapshot) = x11_owner_active_snapshot() {
        return CapturedClipboard::Snapshot(snapshot);
    }

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return CapturedClipboard::Unrestorable;
    };
    let Some(atoms) = X11Atoms::intern(&conn) else {
        return CapturedClipboard::Unrestorable;
    };
    let Some(owner_reply) = conn
        .get_selection_owner(atoms.clipboard)
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return CapturedClipboard::Unrestorable;
    };
    if owner_reply.owner == x11rb::NONE {
        return CapturedClipboard::Empty;
    }

    let root = conn.setup().roots[screen_num].root;
    let Ok(win) = conn.generate_id() else {
        return CapturedClipboard::Unrestorable;
    };
    if conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .is_err()
    {
        return CapturedClipboard::Unrestorable;
    }
    let _ = conn.flush();

    let result = capture_x11_with_window(&conn, win, &atoms);
    let _ = conn.destroy_window(win);
    let _ = conn.flush();
    result
}

fn capture_x11_with_window(
    conn: &RustConnection,
    win: Window,
    atoms: &X11Atoms,
) -> CapturedClipboard<String> {
    let total_deadline = Instant::now() + CAPTURE_TOTAL_TIMEOUT;
    let target_atoms = match request_x11_targets(conn, win, atoms, total_deadline) {
        Some(list) if !list.is_empty() => list,
        _ => return CapturedClipboard::Unrestorable,
    };

    let mut named: Vec<(String, Atom)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for target in target_atoms {
        let Ok(cookie) = conn.get_atom_name(target) else {
            continue;
        };
        let Ok(reply) = cookie.reply() else {
            continue;
        };
        let Ok(name) = String::from_utf8(reply.name) else {
            continue;
        };
        if is_safe_linux_target(&name) && seen.insert(name.clone()) {
            named.push((name, target));
        }
    }
    named.sort_by_key(|(name, _)| linux_capture_priority(name));

    let mut snapshot = LinuxSnapshot::new();
    for (name, target) in named {
        let now = Instant::now();
        if now >= total_deadline {
            break;
        }
        let deadline = (now + TARGET_READ_TIMEOUT).min(total_deadline);
        if let Some(bytes) = request_x11_target_bytes(conn, win, atoms, target, deadline)
            && snapshot.can_fit(bytes.len())
        {
            snapshot.push(name, bytes);
        }
    }

    if snapshot.is_empty() {
        CapturedClipboard::Unrestorable
    } else {
        CapturedClipboard::Snapshot(snapshot)
    }
}

fn request_x11_targets(
    conn: &RustConnection,
    win: Window,
    atoms: &X11Atoms,
    total_deadline: Instant,
) -> Option<Vec<Atom>> {
    let _ = conn.delete_property(win, atoms.transfer);
    conn.convert_selection(
        win,
        atoms.clipboard,
        atoms.targets,
        atoms.transfer,
        x11rb::CURRENT_TIME,
    )
    .ok()?;
    conn.flush().ok()?;

    let deadline = (Instant::now() + TARGET_READ_TIMEOUT).min(total_deadline);
    let prop = wait_selection_notify(conn, win, atoms.targets, deadline)?;
    let reply = conn
        .get_property(true, win, prop, AtomEnum::ANY, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if reply.format != 32 {
        return None;
    }
    Some(
        reply
            .value
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .map(u32::from_ne_bytes)
            .collect(),
    )
}

fn request_x11_target_bytes(
    conn: &RustConnection,
    win: Window,
    atoms: &X11Atoms,
    target: Atom,
    deadline: Instant,
) -> Option<Vec<u8>> {
    let _ = conn.delete_property(win, atoms.transfer);
    conn.convert_selection(
        win,
        atoms.clipboard,
        target,
        atoms.transfer,
        x11rb::CURRENT_TIME,
    )
    .ok()?;
    conn.flush().ok()?;

    let prop = wait_selection_notify(conn, win, target, deadline)?;
    let max_longs = MAX_FORMAT_BYTES.div_ceil(4) as u32;
    let reply = conn
        .get_property(true, win, prop, AtomEnum::ANY, 0, max_longs)
        .ok()?
        .reply()
        .ok()?;
    if reply.type_ == atoms.incr {
        return read_x11_incr(conn, win, prop, deadline);
    }
    if reply.bytes_after > 0 || reply.value.len() > MAX_FORMAT_BYTES {
        return None;
    }
    Some(reply.value)
}

fn wait_selection_notify(
    conn: &RustConnection,
    win: Window,
    target: Atom,
    deadline: Instant,
) -> Option<Atom> {
    let fd = conn.stream().as_raw_fd();
    loop {
        while let Some(event) = conn.poll_for_event().ok()? {
            if let Event::SelectionNotify(ev) = event
                && ev.requestor == win
                && ev.target == target
            {
                return (ev.property != x11rb::NONE).then_some(ev.property);
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let wait_ms = deadline.saturating_duration_since(now).as_millis().min(50) as i32;
        if !poll_fd_readable(fd, wait_ms) {
            continue;
        }
    }
}

fn read_x11_incr(
    conn: &RustConnection,
    win: Window,
    prop: Atom,
    deadline: Instant,
) -> Option<Vec<u8>> {
    let fd = conn.stream().as_raw_fd();
    let max_longs = MAX_FORMAT_BYTES.div_ceil(4) as u32;
    let mut data = Vec::new();
    let _ = conn.flush();

    loop {
        while let Some(event) = conn.poll_for_event().ok()? {
            if let Event::PropertyNotify(ev) = event
                && ev.window == win
                && ev.atom == prop
                && ev.state == Property::NEW_VALUE
            {
                let chunk = conn
                    .get_property(true, win, prop, AtomEnum::ANY, 0, max_longs)
                    .ok()?
                    .reply()
                    .ok()?;
                let _ = conn.flush();
                if chunk.value.is_empty() {
                    return Some(data);
                }
                if data.len().saturating_add(chunk.value.len()) > MAX_FORMAT_BYTES {
                    return None;
                }
                data.extend_from_slice(&chunk.value);
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        let wait_ms = deadline.saturating_duration_since(now).as_millis().min(50) as i32;
        let _ = poll_fd_readable(fd, wait_ms);
    }
}

// ---------------------------------------------------------------------------
// Serveur d'arrière-plan X11 pour servir les formats restaurés (0 % CPU au repos)
// ---------------------------------------------------------------------------

struct X11OwnerCommand {
    snapshot: LinuxSnapshot,
    reply: Sender<Result<(), String>>,
}

struct X11OwnerHandle {
    commands: Mutex<Vec<X11OwnerCommand>>,
    wake_write_fd: RawFd,
    active_snapshot: Arc<Mutex<Option<LinuxSnapshot>>>,
}

static X11_OWNER: OnceLock<Mutex<Option<Arc<X11OwnerHandle>>>> = OnceLock::new();

fn x11_owner_active_snapshot() -> Option<LinuxSnapshot> {
    let cell = X11_OWNER.get()?;
    let guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    let handle = guard.as_ref()?;
    handle
        .active_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

fn restore_x11(snapshot: LinuxSnapshot) -> Result<(), String> {
    let handle = get_or_spawn_x11_owner()?;
    let (tx, rx) = mpsc::channel();
    {
        let mut queue = handle.commands.lock().unwrap_or_else(|e| e.into_inner());
        queue.push(X11OwnerCommand {
            snapshot,
            reply: tx,
        });
    }
    wake_pipe(handle.wake_write_fd);
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "délai dépassé pour la restauration X11".to_owned())?
}

fn get_or_spawn_x11_owner() -> Result<Arc<X11OwnerHandle>, String> {
    let cell = X11_OWNER.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handle) = guard.as_ref() {
        return Ok(handle.clone());
    }

    let (read_fd, write_fd) = create_wake_pipe()?;
    let active_snapshot = Arc::new(Mutex::new(None));
    let handle = Arc::new(X11OwnerHandle {
        commands: Mutex::new(Vec::new()),
        wake_write_fd: write_fd,
        active_snapshot,
    });

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let thread_handle = handle.clone();
    thread::Builder::new()
        .name("vtt-clip-x11".into())
        .spawn(move || x11_owner_loop(thread_handle, read_fd, ready_tx))
        .map_err(|e| format!("thread presse-papier X11 : {e}"))?;

    ready_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "initialisation du presse-papier X11 trop longue".to_owned())??;

    *guard = Some(handle.clone());
    Ok(handle)
}

struct IncrTransfer {
    requestor: Window,
    property: Atom,
    target: Atom,
    data: Arc<[u8]>,
    offset: usize,
    started: Instant,
}

struct OwnedState {
    entries: HashMap<Atom, Arc<[u8]>>,
    targets_list: Vec<Atom>,
    timestamp: u32,
    incr_transfers: Vec<IncrTransfer>,
}

fn x11_owner_loop(
    handle: Arc<X11OwnerHandle>,
    wake_read_fd: RawFd,
    ready: Sender<Result<(), String>>,
) {
    let (conn, screen_num) = match x11rb::connect(None) {
        Ok(pair) => pair,
        Err(e) => {
            let _ = ready.send(Err(format!("connexion X11 : {e}")));
            return;
        }
    };
    let Some(atoms) = X11Atoms::intern(&conn) else {
        let _ = ready.send(Err("atomes X11 indisponibles".into()));
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    let Ok(win) = conn.generate_id() else {
        let _ = ready.send(Err("fenêtre X11 impossible".into()));
        return;
    };
    if let Err(e) = conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new(),
    ) {
        let _ = ready.send(Err(format!("fenêtre presse-papier X11 : {e}")));
        return;
    }
    let _ = conn.flush();
    let _ = ready.send(Ok(()));

    let x11_fd = conn.stream().as_raw_fd();
    let mut state = OwnedState {
        entries: HashMap::new(),
        targets_list: Vec::new(),
        timestamp: x11rb::CURRENT_TIME,
        incr_transfers: Vec::new(),
    };

    loop {
        // Traite d'abord toutes les commandes reçues.
        let pending: Vec<X11OwnerCommand> = {
            let mut queue = handle.commands.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *queue)
        };
        for cmd in pending {
            let res = apply_x11_restore(&conn, win, &atoms, cmd.snapshot, &mut state, &handle);
            let _ = cmd.reply.send(res);
        }

        // Traite tous les événements X11 déjà en mémoire tampon.
        loop {
            match conn.poll_for_event() {
                Ok(Some(event)) => handle_x11_event(&conn, win, &atoms, event, &mut state, &handle),
                Ok(None) => break,
                Err(_) => return,
            }
        }
        let _ = conn.flush();

        // Purge les transferts INCR expirés.
        state
            .incr_transfers
            .retain(|t| t.started.elapsed() < X11_INCR_TTL);

        // Attente passive sur la socket X11 et le tube de réveil.
        let timeout_ms = if state.incr_transfers.is_empty() {
            -1
        } else {
            1000
        };
        let mut fds = [
            libc::pollfd {
                fd: x11_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake_read_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: `fds` est un tableau valide de deux `pollfd`.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if rc < 0 {
            continue;
        }
        if (fds[1].revents & libc::POLLIN) != 0 {
            drain_pipe(wake_read_fd);
        }
        if (fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
            return;
        }
    }
}

fn apply_x11_restore(
    conn: &RustConnection,
    win: Window,
    atoms: &X11Atoms,
    snapshot: LinuxSnapshot,
    state: &mut OwnedState,
    handle: &X11OwnerHandle,
) -> Result<(), String> {
    let mut entries = HashMap::new();
    let mut targets_list = vec![atoms.targets, atoms.timestamp, atoms.save_targets];
    for entry in snapshot.entries() {
        let Ok(cookie) = conn.intern_atom(false, entry.format.as_bytes()) else {
            continue;
        };
        let Ok(reply) = cookie.reply() else {
            continue;
        };
        let atom = reply.atom;
        entries.insert(atom, Arc::<[u8]>::from(entry.data.as_slice()));
        targets_list.push(atom);
    }
    if entries.is_empty() {
        return Err("aucun format X11 valide à restaurer".into());
    }

    conn.set_selection_owner(win, atoms.clipboard, x11rb::CURRENT_TIME)
        .map_err(|e| format!("set_selection_owner : {e}"))?;
    let owner = conn
        .get_selection_owner(atoms.clipboard)
        .map_err(|e| format!("get_selection_owner : {e}"))?
        .reply()
        .map_err(|e| format!("get_selection_owner : {e}"))?
        .owner;
    if owner != win {
        return Err("impossible de reprendre la sélection CLIPBOARD".into());
    }

    state.entries = entries;
    state.targets_list = targets_list;
    state.timestamp = x11rb::CURRENT_TIME;
    state.incr_transfers.clear();
    *handle
        .active_snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(snapshot);
    Ok(())
}

fn handle_x11_event(
    conn: &RustConnection,
    win: Window,
    atoms: &X11Atoms,
    event: Event,
    state: &mut OwnedState,
    handle: &X11OwnerHandle,
) {
    match event {
        Event::SelectionClear(ev) if ev.selection == atoms.clipboard => {
            state.entries.clear();
            state.targets_list.clear();
            state.incr_transfers.clear();
            *handle
                .active_snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
        Event::SelectionRequest(req) if req.selection == atoms.clipboard => {
            serve_selection_request(conn, win, atoms, &req, state);
        }
        Event::PropertyNotify(ev) if ev.state == Property::DELETE => {
            advance_incr_transfer(conn, ev.window, ev.atom, state);
        }
        _ => {}
    }
}

fn serve_selection_request(
    conn: &RustConnection,
    _win: Window,
    atoms: &X11Atoms,
    req: &SelectionRequestEvent,
    state: &mut OwnedState,
) {
    let prop = if req.property == x11rb::NONE {
        req.target
    } else {
        req.property
    };

    let succeeded = if req.target == atoms.multiple {
        serve_multiple(conn, atoms, req.requestor, prop, state)
    } else {
        serve_single_target(conn, atoms, req.requestor, req.target, prop, state)
    };

    let notify = SelectionNotifyEvent {
        response_type: xproto::SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if succeeded { prop } else { x11rb::NONE },
    };
    let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify);
    let _ = conn.flush();
}

fn serve_multiple(
    conn: &RustConnection,
    atoms: &X11Atoms,
    requestor: Window,
    prop: Atom,
    state: &mut OwnedState,
) -> bool {
    if prop == x11rb::NONE {
        return false;
    }
    let Some(reply) = conn
        .get_property(false, requestor, prop, atoms.atom_pair, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return false;
    };
    if reply.format != 32 {
        return false;
    }
    let mut pair_atoms: Vec<Atom> = reply
        .value
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_ne_bytes)
        .collect();
    for pair in pair_atoms.as_chunks_mut::<2>().0 {
        let target = pair[0];
        let dest_prop = pair[1];
        if dest_prop == x11rb::NONE
            || !serve_single_target(conn, atoms, requestor, target, dest_prop, state)
        {
            pair[1] = x11rb::NONE;
        }
    }
    let _ = conn.change_property32(
        PropMode::REPLACE,
        requestor,
        prop,
        atoms.atom_pair,
        &pair_atoms,
    );
    true
}

fn serve_single_target(
    conn: &RustConnection,
    atoms: &X11Atoms,
    requestor: Window,
    target: Atom,
    prop: Atom,
    state: &mut OwnedState,
) -> bool {
    if target == atoms.targets {
        return conn
            .change_property32(
                PropMode::REPLACE,
                requestor,
                prop,
                AtomEnum::ATOM,
                &state.targets_list,
            )
            .is_ok();
    }
    if target == atoms.timestamp {
        return conn
            .change_property32(
                PropMode::REPLACE,
                requestor,
                prop,
                AtomEnum::INTEGER,
                &[state.timestamp],
            )
            .is_ok();
    }
    if target == atoms.save_targets {
        return conn
            .change_property8(PropMode::REPLACE, requestor, prop, AtomEnum::NONE, &[])
            .is_ok();
    }

    let Some(data) = state.entries.get(&target).cloned() else {
        return false;
    };

    if data.len() <= X11_INCR_CHUNK_BYTES {
        return conn
            .change_property8(PropMode::REPLACE, requestor, prop, target, &data)
            .is_ok();
    }

    // Transfert incrémental INCR (images volumineuses) : demander les
    // notifications de suppression de propriété sur la fenêtre cliente.
    let _ = conn.change_window_attributes(
        requestor,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    );
    let total_len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    if conn
        .change_property32(PropMode::REPLACE, requestor, prop, atoms.incr, &[total_len])
        .is_err()
    {
        return false;
    }
    state.incr_transfers.push(IncrTransfer {
        requestor,
        property: prop,
        target,
        data,
        offset: 0,
        started: Instant::now(),
    });
    true
}

fn advance_incr_transfer(
    conn: &RustConnection,
    window: Window,
    atom: Atom,
    state: &mut OwnedState,
) {
    let Some(pos) = state
        .incr_transfers
        .iter()
        .position(|t| t.requestor == window && t.property == atom)
    else {
        return;
    };
    let transfer = &mut state.incr_transfers[pos];
    let finished = if transfer.offset >= transfer.data.len() {
        let _ = conn.change_property8(
            PropMode::REPLACE,
            transfer.requestor,
            transfer.property,
            transfer.target,
            &[],
        );
        true
    } else {
        let end = (transfer.offset + X11_INCR_CHUNK_BYTES).min(transfer.data.len());
        let chunk = &transfer.data[transfer.offset..end];
        let ok = conn
            .change_property8(
                PropMode::REPLACE,
                transfer.requestor,
                transfer.property,
                transfer.target,
                chunk,
            )
            .is_ok();
        transfer.offset = end;
        !ok
    };
    let _ = conn.flush();
    if finished {
        state.incr_transfers.remove(pos);
    }
}

// ---------------------------------------------------------------------------
// Utilitaires POSIX (poll & tube de réveil)
// ---------------------------------------------------------------------------

fn poll_fd_readable(fd: RawFd, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `pfd` est un descripteur valide unique.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    rc > 0 && (pfd.revents & (libc::POLLIN | libc::POLLHUP)) != 0
}

fn create_wake_pipe() -> Result<(RawFd, RawFd), String> {
    let mut fds = [0 as RawFd; 2];
    // SAFETY: tableau valide de 2 descripteurs ; O_NONBLOCK + O_CLOEXEC.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        return Err("création du tube de réveil impossible".into());
    }
    Ok((fds[0], fds[1]))
}

fn wake_pipe(fd: RawFd) {
    let byte = [1u8; 1];
    // SAFETY: écriture non bloquante d'un octet sur le tube de réveil.
    let _ = unsafe { libc::write(fd, byte.as_ptr().cast(), 1) };
}

fn drain_pipe(fd: RawFd) {
    let mut buf = [0u8; 64];
    loop {
        // SAFETY: lecture non bloquante du tube de réveil jusqu'à EAGAIN.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static X11_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn read_x11_from_external_client() -> Option<LinuxSnapshot> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let atoms = X11Atoms::intern(&conn)?;
        let root = conn.setup().roots[screen_num].root;
        let win = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .ok()?;
        conn.flush().ok()?;
        let read_back = capture_x11_with_window(&conn, win, &atoms).into_snapshot();
        let _ = conn.destroy_window(win);
        read_back
    }

    fn entry_bytes<'a>(snap: &'a LinuxSnapshot, mime: &str) -> Option<&'a [u8]> {
        snap.entries()
            .iter()
            .find(|e| e.format == mime)
            .map(|e| e.data.as_slice())
    }

    /// Vérifie la capture et la restauration multi-formats (HTML, PNG, liste
    /// de fichiers, texte) ainsi que les gros transferts `INCR` (> 64 Kio) sur
    /// un vrai serveur X11 (lancé sous `xvfb-run` ou dans une session X11).
    /// Si aucun `DISPLAY` n'est disponible (CI sans serveur X), le test passe
    /// proprement sans échouer.
    #[test]
    fn x11_round_trip_multiformat_and_incr() {
        if std::env::var_os("DISPLAY").is_none() || wayland_session() {
            return;
        }
        let _lock = X11_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let html = b"<p>Bonjour <b>monde</b></p>".to_vec();
        let uri_list = b"file:///tmp/rapport.pdf\r\nfile:///tmp/photo.png\r\n".to_vec();
        // Image synthétique > 200 Kio pour forcer le protocole X11 INCR.
        let mut large_png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        large_png.extend((0..200_000u32).map(|i| (i % 251) as u8));

        let mut initial = LinuxSnapshot::new();
        assert!(initial.push("text/html".into(), html.clone()));
        assert!(initial.push("text/uri-list".into(), uri_list.clone()));
        assert!(initial.push("image/png".into(), large_png.clone()));
        assert!(initial.push("UTF8_STRING".into(), b"Bonjour monde".to_vec()));

        restore_x11(initial).expect("restauration X11 initiale");

        let read_back = read_x11_from_external_client().expect("capture X11 non vide");
        assert_eq!(entry_bytes(&read_back, "text/html"), Some(html.as_slice()));
        assert_eq!(
            entry_bytes(&read_back, "text/uri-list"),
            Some(uri_list.as_slice())
        );
        assert_eq!(
            entry_bytes(&read_back, "UTF8_STRING"),
            Some(b"Bonjour monde".as_slice())
        );
        assert_eq!(
            entry_bytes(&read_back, "image/png"),
            Some(large_png.as_slice())
        );
    }

    /// Vérifie la restauration différée après remplacement par le texte dicté,
    /// et l'abandon de la restauration si l'utilisateur copie autre chose
    /// pendant le délai.
    #[test]
    fn x11_delayed_restore_and_user_copy_guard() {
        if std::env::var_os("DISPLAY").is_none() || wayland_session() {
            return;
        }
        let _lock = X11_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4, 5];
        let html = b"<em>important</em>".to_vec();
        let mut orig = LinuxSnapshot::new();
        orig.push("image/png".into(), png.clone());
        orig.push("text/html".into(), html.clone());

        // Simule le collage d'un texte dicté via un propriétaire temporaire.
        let mut dictated = LinuxSnapshot::new();
        dictated.push("UTF8_STRING".into(), b"texte dicte".to_vec());
        restore_x11(dictated).expect("pose texte dicté");

        schedule_restore(
            orig.clone(),
            "texte dicte".into(),
            Duration::from_millis(50),
            || {
                read_x11_from_external_client().and_then(|s| {
                    entry_bytes(&s, "UTF8_STRING")
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .map(str::to_owned)
                })
            },
        );
        thread::sleep(Duration::from_millis(200));

        let restored = read_x11_from_external_client().expect("restauré");
        assert_eq!(entry_bytes(&restored, "image/png"), Some(png.as_slice()));
        assert_eq!(entry_bytes(&restored, "text/html"), Some(html.as_slice()));

        // Cas 2 : l'utilisateur copie autre chose avant le délai → ne rien écraser.
        let mut dictated2 = LinuxSnapshot::new();
        dictated2.push("UTF8_STRING".into(), b"temporaire".to_vec());
        restore_x11(dictated2).expect("pose texte temporaire");
        schedule_restore(
            orig,
            "temporaire".into(),
            Duration::from_millis(120),
            || {
                read_x11_from_external_client().and_then(|s| {
                    entry_bytes(&s, "UTF8_STRING")
                        .and_then(|b| std::str::from_utf8(b).ok())
                        .map(str::to_owned)
                })
            },
        );

        let mut user_copy = LinuxSnapshot::new();
        user_copy.push("UTF8_STRING".into(), b"copie utilisateur".to_vec());
        restore_x11(user_copy).expect("copie utilisateur");

        thread::sleep(Duration::from_millis(220));
        let after = read_x11_from_external_client().expect("copie conservée");
        assert_eq!(entry_bytes(&after, "image/png"), None);
        assert_eq!(
            entry_bytes(&after, "UTF8_STRING"),
            Some(b"copie utilisateur".as_slice())
        );
    }
}
