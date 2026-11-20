//! Synchronisation locale Wi-Fi côté bureau : serveur TCP, découverte mDNS,
//! génération de la matrice QR d'appairage et commandes Tauri pour l'écran
//! **Réglages → Synchronisation**.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use qrcode::{Color, EcLevel, QrCode};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use vtt_core::sync::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_PORT, DeviceId, DirectDecisionHandle, DiscoveredPeer,
    DiskSyncStorage, MdnsService, PairingOffer, Peer, PendingDirectPairing, SyncEvent, SyncOptions,
    SyncServer, SyncStorage, SyncSummary, detect_local_addrs, now_ms, pair_over_tcp,
    start_direct_pairing_over_tcp, sync_with_peer,
};

use crate::app_settings;
use crate::costs::{self, LedgerStore};
use crate::paths::Paths;

/// Événement Tauri émis à la fenêtre dès qu'un appairage ou une synchro
/// modifie l'état local (permet de rafraîchir l'écran en direct).
pub const SYNC_CHANGED_EVENT: &str = "sync-changed";

/// Matrice QR code compacte (`modules` contient `size * size` caractères `'1'`/`'0'`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QrMatrix {
    pub size: usize,
    pub modules: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingOfferView {
    pub uri: String,
    pub expires_at_ms: u64,
    pub fingerprint: String,
    pub addrs: Vec<String>,
    pub qr: QrMatrix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingDirectPairingView {
    pub session_id: String,
    pub remote_device_id: String,
    pub remote_device_name: String,
    pub code: String,
    pub formatted_code: String,
    pub expires_at_ms: u64,
    pub incoming: bool,
    pub local_approved: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredPeerView {
    pub device_id: String,
    pub device_name: String,
    pub addr: String,
    pub already_paired: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerView {
    pub device_id: String,
    pub device_name: String,
    pub fingerprint: String,
    pub paired_at_ms: u64,
    pub last_sync_ms: Option<u64>,
    pub last_addr: Option<String>,
    pub online_addr: Option<String>,
    pub sync_settings: bool,
    pub sync_meetings: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncActivity {
    pub message: String,
    pub at_ms: u64,
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncView {
    pub listening: bool,
    pub port: Option<u16>,
    pub local_addrs: Vec<String>,
    pub mdns_active: bool,
    pub mdns_error: Option<String>,
    pub server_error: Option<String>,
    pub device_id: String,
    pub device_name: String,
    pub direct_pairing_enabled: bool,
    pub active_offer: Option<PairingOfferView>,
    pub pending_direct: Option<PendingDirectPairingView>,
    pub discovered: Vec<DiscoveredPeerView>,
    pub peers: Vec<PeerView>,
    pub last_activity: Option<SyncActivity>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncSummaryView {
    pub peer_id: String,
    pub peer_name: String,
    pub config_changed: bool,
    pub meetings_sent: usize,
    pub meetings_received: usize,
    pub usage_changed: bool,
    pub message: String,
    pub at_ms: u64,
}

struct SyncRuntime {
    server: Option<SyncServer>,
    mdns: Option<MdnsService>,
    server_error: Option<String>,
    mdns_error: Option<String>,
}

struct OutgoingDirectSlot {
    pending: PendingDirectPairing,
    decision: DirectDecisionHandle,
    local_approved: bool,
}

pub struct SyncService {
    app: AppHandle,
    paths: Paths,
    storage: Arc<DiskSyncStorage>,
    runtime: Mutex<SyncRuntime>,
    last_activity: Arc<Mutex<Option<SyncActivity>>>,
    direct_pairing_wanted: Arc<AtomicBool>,
    outgoing_direct: Arc<Mutex<Option<OutgoingDirectSlot>>>,
    incoming_approved_session: Arc<Mutex<Option<String>>>,
}

pub type SyncState = Arc<SyncService>;

impl SyncService {
    /// Initialise le service de synchronisation.
    ///
    /// Pour ne jamais déclencher d'alerte pare-feu inutile sur un PC qui n'utilise
    /// pas la synchro Wi-Fi, l'écoute ne démarre automatiquement au lancement que
    /// si au moins un appareil est déjà appairé dans `sync_peers.json`.
    pub fn new(app: AppHandle, paths: Paths) -> Arc<Self> {
        let storage = Arc::new(
            DiskSyncStorage::new(
                paths.config_file.clone(),
                paths.sync_peers_file.clone(),
                paths.meetings_dir.clone(),
                default_device_name(),
            )
            .with_usage_path(paths.usage_file.clone()),
        );
        let has_peers = paths.sync_peers_file.exists()
            && storage.peers().map(|p| !p.is_empty()).unwrap_or(false);

        let service = Arc::new(Self {
            app,
            paths,
            storage,
            runtime: Mutex::new(SyncRuntime {
                server: None,
                mdns: None,
                server_error: None,
                mdns_error: None,
            }),
            last_activity: Arc::new(Mutex::new(None)),
            direct_pairing_wanted: Arc::new(AtomicBool::new(false)),
            outgoing_direct: Arc::new(Mutex::new(None)),
            incoming_approved_session: Arc::new(Mutex::new(None)),
        });

        if has_peers && let Err(e) = service.start_listening() {
            log::warn!("sync: démarrage automatique de l'écoute locale échoué : {e}");
        }
        service
    }

    pub fn start_listening(&self) -> Result<(), String> {
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(server) = rt.server.as_ref() {
            if self.direct_pairing_wanted.load(Ordering::SeqCst) {
                server.set_direct_pairing_enabled(true);
            }
            return Ok(());
        }

        let app = self.app.clone();
        let paths = self.paths.clone();
        let last_activity = Arc::clone(&self.last_activity);
        let incoming_approved = Arc::clone(&self.incoming_approved_session);
        let on_event: Arc<dyn Fn(SyncEvent) + Send + Sync> = Arc::new(move |event: SyncEvent| {
            handle_server_event(&app, &paths, &last_activity, &incoming_approved, event);
        });
        let storage: Arc<dyn SyncStorage> = self.storage.clone();

        let preferred = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_PORT);
        let server = match SyncServer::start(
            preferred,
            Arc::clone(&storage),
            Some(Arc::clone(&on_event)),
        ) {
            Ok(s) => s,
            Err(first_err) => {
                log::warn!(
                    "sync: port {DEFAULT_PORT} indisponible ({first_err}), essai sur un port libre"
                );
                let fallback = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
                match SyncServer::start(fallback, storage, Some(on_event)) {
                    Ok(s) => s,
                    Err(e) => {
                        let msg = e.to_string();
                        rt.server_error = Some(msg.clone());
                        return Err(msg);
                    }
                }
            }
        };

        if self.direct_pairing_wanted.load(Ordering::SeqCst) {
            server.set_direct_pairing_enabled(true);
        }
        let port = server.port();
        rt.server_error = None;
        rt.server = Some(server);

        match self
            .storage
            .identity()
            .and_then(|id| MdnsService::start(id, port, None))
        {
            Ok(mdns) => {
                rt.mdns = Some(mdns);
                rt.mdns_error = None;
            }
            Err(e) => {
                log::warn!("sync: découverte mDNS indisponible ({e}) — secours IP:port actif");
                rt.mdns = None;
                rt.mdns_error = Some(e.to_string());
            }
        }
        log::info!("sync: écoute locale démarrée sur le port {port}");
        Ok(())
    }

    pub fn stop_listening(&self) {
        self.reject_outgoing_direct();
        *self
            .incoming_approved_session
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mdns) = rt.mdns.take() {
            mdns.stop();
        }
        if let Some(server) = rt.server.take() {
            server.stop();
            log::info!("sync: écoute locale arrêtée");
        }
        rt.server_error = None;
        rt.mdns_error = None;
    }

    fn reject_outgoing_direct(&self) {
        let mut guard = self
            .outgoing_direct
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = guard.take() {
            slot.decision.reject();
        }
    }

    fn active_outgoing_direct(&self) -> Option<PendingDirectPairingView> {
        let mut guard = self
            .outgoing_direct
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|slot| now_ms() >= slot.pending.expires_at_ms)
            && let Some(old) = guard.take()
        {
            old.decision.reject();
        }
        guard
            .as_ref()
            .map(|slot| pending_to_view(&slot.pending, slot.local_approved))
    }

    fn restart_mdns_if_running(&self) {
        let mut rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        let Some(port) = rt.server.as_ref().map(SyncServer::port) else {
            return;
        };
        if let Some(old) = rt.mdns.take() {
            old.stop();
        }
        match self
            .storage
            .identity()
            .and_then(|id| MdnsService::start(id, port, None))
        {
            Ok(mdns) => {
                rt.mdns = Some(mdns);
                rt.mdns_error = None;
            }
            Err(e) => {
                rt.mdns_error = Some(e.to_string());
            }
        }
    }

    pub fn view(&self) -> Result<SyncView, String> {
        let identity = self.storage.identity().map_err(|e| e.to_string())?;
        let peers = self.storage.peers().map_err(|e| e.to_string())?;

        let (
            listening,
            port,
            direct_pairing_enabled,
            active_offer,
            incoming_direct,
            discovered_raw,
            mdns_active,
            mdns_error,
            server_error,
        ) = {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            let port = rt.server.as_ref().map(SyncServer::port);
            let direct_enabled = rt
                .server
                .as_ref()
                .is_some_and(SyncServer::is_direct_pairing_enabled);
            let active_offer = rt
                .server
                .as_ref()
                .and_then(SyncServer::active_pairing)
                .and_then(|o| offer_to_view(&o).ok());
            let incoming_direct = rt
                .server
                .as_ref()
                .and_then(SyncServer::pending_direct_pairing);
            let discovered_raw = rt.mdns.as_ref().map(MdnsService::peers).unwrap_or_default();
            (
                rt.server.is_some(),
                port,
                direct_enabled,
                active_offer,
                incoming_direct,
                discovered_raw,
                rt.mdns.is_some(),
                rt.mdns_error.clone(),
                rt.server_error.clone(),
            )
        };

        let pending_direct = self.active_outgoing_direct().or_else(|| {
            incoming_direct.map(|pending| {
                let approved = self
                    .incoming_approved_session
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_deref()
                    == Some(pending.session_id.as_str());
                pending_to_view(&pending, approved)
            })
        });

        let local_addrs = port.map(detect_local_addrs).unwrap_or_default();
        let (peer_views, discovered_views) = build_peer_views(&peers, &discovered_raw);
        let last_activity = self
            .last_activity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        Ok(SyncView {
            listening,
            port,
            local_addrs,
            mdns_active,
            mdns_error,
            server_error,
            device_id: identity.device_id.as_str().to_owned(),
            device_name: identity.device_name,
            direct_pairing_enabled,
            active_offer,
            pending_direct,
            discovered: discovered_views,
            peers: peer_views,
            last_activity,
        })
    }

    /// Active ou désactive l'acceptation des demandes d'appairage direct par
    /// code à 6 chiffres (activée uniquement tant que la page Synchronisation
    /// est ouverte).
    pub fn set_direct_pairing_enabled(&self, enabled: bool) -> Result<SyncView, String> {
        self.direct_pairing_wanted.store(enabled, Ordering::SeqCst);
        {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(server) = rt.server.as_ref() {
                server.set_direct_pairing_enabled(enabled);
            }
        }
        if !enabled {
            self.reject_outgoing_direct();
            *self
                .incoming_approved_session
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
        self.view()
    }

    /// Lance un appairage direct vers un appareil détecté : retourne aussitôt
    /// le code à 6 chiffres à vérifier visuellement sur les deux écrans.
    pub fn start_direct_pairing(
        self: &Arc<Self>,
        device_id: Option<&str>,
        addr: &str,
    ) -> Result<PendingDirectPairingView, String> {
        let trimmed_addr = addr.trim();
        if trimmed_addr.is_empty() {
            return Err("adresse de l'appareil cible vide".into());
        }
        let expected_remote = match device_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => Some(DeviceId::parse(id).map_err(|e| e.to_string())?),
            None => None,
        };

        if self.active_outgoing_direct().is_some() {
            return Err("un appairage direct est déjà en cours".into());
        }
        let local_listen_port = {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if rt
                .server
                .as_ref()
                .and_then(SyncServer::pending_direct_pairing)
                .is_some()
            {
                return Err("une demande d'appairage attend déjà votre confirmation".into());
            }
            rt.server.as_ref().map(SyncServer::port)
        };

        let storage: Arc<dyn SyncStorage> = self.storage.clone();
        let handle = start_direct_pairing_over_tcp(
            storage,
            &[trimmed_addr.to_owned()],
            expected_remote.as_ref(),
            local_listen_port,
            DEFAULT_CONNECT_TIMEOUT,
            None,
        )
        .map_err(|e| e.to_string())?;

        let pending = handle.pending().clone();
        let wait_session_id = pending.session_id.clone();
        let decision = handle.decision_handle();
        {
            let mut guard = self
                .outgoing_direct
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *guard = Some(OutgoingDirectSlot {
                pending: pending.clone(),
                decision,
                local_approved: false,
            });
        }

        let service = Arc::clone(self);
        if let Err(e) = thread::Builder::new()
            .name("vtt-sync-direct-wait".into())
            .spawn(move || {
                let res = handle.wait();
                {
                    let mut guard = service
                        .outgoing_direct
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if guard
                        .as_ref()
                        .is_some_and(|slot| slot.pending.session_id == wait_session_id)
                    {
                        *guard = None;
                    }
                }
                match res {
                    Ok(peer) => {
                        let _ = service.start_listening();
                        service.record_activity(
                            format!("Appareil « {} » appairé.", peer.device_name),
                            false,
                        );
                    }
                    Err(e) => {
                        service.record_activity(format!("Appairage direct annulé : {e}"), true);
                    }
                }
                let _ = service.app.emit(SYNC_CHANGED_EVENT, ());
            })
        {
            self.reject_outgoing_direct();
            return Err(format!(
                "impossible de démarrer l'attente d'appairage : {e}"
            ));
        }

        let _ = self.app.emit(SYNC_CHANGED_EVENT, ());
        Ok(pending_to_view(&pending, false))
    }

    /// Approuve ou refuse une demande d'appairage direct (entrante ou sortante).
    pub fn respond_direct_pairing(
        &self,
        session_id: &str,
        approve: bool,
    ) -> Result<SyncView, String> {
        let mut handled_outgoing = false;
        {
            let mut guard = self
                .outgoing_direct
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = guard.as_mut()
                && slot.pending.session_id == session_id
            {
                slot.decision.decide(approve);
                if approve {
                    slot.local_approved = true;
                } else {
                    *guard = None;
                }
                handled_outgoing = true;
            }
        }
        if handled_outgoing {
            let _ = self.app.emit(SYNC_CHANGED_EVENT, ());
            return self.view();
        }

        {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            let server = rt
                .server
                .as_ref()
                .ok_or_else(|| "serveur de synchronisation arrêté".to_owned())?;
            server
                .respond_direct_pairing(session_id, approve)
                .map_err(|e| e.to_string())?;
        }
        {
            let mut approved_guard = self
                .incoming_approved_session
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if approve {
                *approved_guard = Some(session_id.to_owned());
            } else if approved_guard.as_deref() == Some(session_id) {
                *approved_guard = None;
            }
        }
        let _ = self.app.emit(SYNC_CHANGED_EVENT, ());
        self.view()
    }

    pub fn open_pairing(&self) -> Result<PairingOfferView, String> {
        self.start_listening()?;
        let offer = {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            let server = rt
                .server
                .as_ref()
                .ok_or_else(|| "serveur de synchronisation arrêté".to_owned())?;
            server.open_pairing_auto(None).map_err(|e| e.to_string())?
        };
        offer_to_view(&offer)
    }

    pub fn cancel_pairing(&self) {
        let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(server) = rt.server.as_ref() {
            server.cancel_pairing();
        }
    }

    pub fn refresh_discovery(&self) -> Result<SyncView, String> {
        {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(mdns) = rt.mdns.as_ref() {
                let _ = mdns.refresh();
            }
        }
        self.view()
    }

    pub fn rename_device(&self, name: &str) -> Result<SyncView, String> {
        self.storage
            .rename_local_device(name)
            .map_err(|e| e.to_string())?;
        self.restart_mdns_if_running();
        self.view()
    }

    pub fn pair_uri(&self, uri: &str) -> Result<PeerView, String> {
        let offer = PairingOffer::from_uri(uri.trim()).map_err(|e| e.to_string())?;
        let peer = pair_over_tcp(self.storage.as_ref(), &offer, DEFAULT_CONNECT_TIMEOUT)
            .map_err(|e| e.to_string())?;
        // Une fois un premier pair ajouté, on active l'écoute locale pour
        // recevoir aussi les synchronisations initiées par cet appareil.
        let _ = self.start_listening();
        self.record_activity(format!("Appareil « {} » appairé.", peer.device_name), false);
        let _ = self.app.emit(SYNC_CHANGED_EVENT, ());
        Ok(peer_to_view(&peer, None))
    }

    pub fn sync_now(
        &self,
        device_id: &str,
        addr_override: Option<&str>,
    ) -> Result<SyncSummaryView, String> {
        let id = DeviceId::parse(device_id).map_err(|e| e.to_string())?;
        let manual = addr_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let discovered_addr = if manual.is_none() {
            let rt = self.runtime.lock().unwrap_or_else(|e| e.into_inner());
            rt.mdns.as_ref().and_then(|mdns| {
                mdns.peers()
                    .into_iter()
                    .find(|d| d.device_id == id)
                    .map(|d| d.addr.to_string())
            })
        } else {
            None
        };
        let target = manual.as_deref().or(discovered_addr.as_deref());

        let summary = sync_with_peer(
            self.storage.as_ref(),
            &id,
            target,
            SyncOptions::default(),
            DEFAULT_CONNECT_TIMEOUT,
        )
        .map_err(|e| {
            self.record_activity(e.to_string(), true);
            e.to_string()
        })?;

        if summary.config_changed {
            apply_synced_config(&self.app, &self.paths);
        }
        if summary.usage_changed {
            reload_synced_usage(&self.app);
        }
        let message = format_summary_message(&summary);
        self.record_activity(message.clone(), false);
        let _ = self.app.emit(SYNC_CHANGED_EVENT, ());

        Ok(SyncSummaryView {
            peer_id: summary.peer_id.as_str().to_owned(),
            peer_name: summary.peer_name,
            config_changed: summary.config_changed,
            usage_changed: summary.usage_changed,
            meetings_sent: summary.meetings_sent,
            meetings_received: summary.meetings_received,
            message,
            at_ms: summary.at_ms,
        })
    }

    pub fn set_peer_prefs(
        &self,
        device_id: &str,
        sync_settings: bool,
        sync_meetings: bool,
    ) -> Result<bool, String> {
        let id = DeviceId::parse(device_id).map_err(|e| e.to_string())?;
        self.storage
            .set_peer_preferences(&id, sync_settings, sync_meetings)
            .map_err(|e| e.to_string())
    }

    pub fn remove_peer(&self, device_id: &str) -> Result<bool, String> {
        let id = DeviceId::parse(device_id).map_err(|e| e.to_string())?;
        let removed = self.storage.remove_peer(&id).map_err(|e| e.to_string())?;
        if removed {
            self.record_activity("Appareil oublié et clé révoquée.".into(), false);
            let _ = self.app.emit(SYNC_CHANGED_EVENT, ());
        }
        Ok(removed)
    }

    fn record_activity(&self, message: String, is_error: bool) {
        let mut guard = self.last_activity.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(SyncActivity {
            message,
            at_ms: now_ms(),
            is_error,
        });
    }
}

fn handle_server_event(
    app: &AppHandle,
    paths: &Paths,
    last_activity: &Mutex<Option<SyncActivity>>,
    incoming_approved: &Mutex<Option<String>>,
    event: SyncEvent,
) {
    let activity = match event {
        SyncEvent::Paired { peer } => {
            *incoming_approved.lock().unwrap_or_else(|e| e.into_inner()) = None;
            SyncActivity {
                message: format!("Appareil « {} » appairé.", peer.device_name),
                at_ms: peer.paired_at_ms,
                is_error: false,
            }
        }
        SyncEvent::Synced { summary } => {
            if summary.config_changed {
                apply_synced_config(app, paths);
            }
            if summary.usage_changed {
                reload_synced_usage(app);
            }
            SyncActivity {
                message: format_summary_message(&summary),
                at_ms: summary.at_ms,
                is_error: false,
            }
        }
        SyncEvent::DirectPairingRequested { pending } => SyncActivity {
            message: format!(
                "Demande d'appairage de « {} » (code {}).",
                pending.remote_device_name,
                pending.formatted_code()
            ),
            at_ms: now_ms(),
            is_error: false,
        },
        SyncEvent::DirectPairingEnded { session_id, reason } => {
            let mut guard = incoming_approved.lock().unwrap_or_else(|e| e.into_inner());
            if guard.as_deref() == Some(session_id.as_str()) {
                *guard = None;
            }
            SyncActivity {
                message: reason
                    .map(|r| format!("Appairage direct interrompu : {r}"))
                    .unwrap_or_else(|| "Appairage direct annulé.".into()),
                at_ms: now_ms(),
                is_error: true,
            }
        }
        SyncEvent::ConnectionFailed { reason } => SyncActivity {
            message: format!("Connexion entrante refusée : {reason}"),
            at_ms: now_ms(),
            is_error: true,
        },
    };
    *last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Some(activity);
    let _ = app.emit(SYNC_CHANGED_EVENT, ());
}

fn apply_synced_config(app: &AppHandle, paths: &Paths) {
    match paths.load_config() {
        (config, None) => {
            let notes = app_settings::apply(app, paths, &config);
            if !notes.is_empty() {
                log::info!("sync: réglages appliqués ({})", notes.join(" ; "));
            }
        }
        (_, Some(err)) => {
            log::warn!("sync: rechargement config.toml impossible : {err}");
        }
    }
}

fn reload_synced_usage(app: &AppHandle) {
    if let Some(ledger) = app.try_state::<LedgerStore>() {
        costs::reload_remote(&ledger);
        log::info!("sync: statistiques des autres appareils rechargées");
    }
}

/// Nom proposé par défaut pour cet appareil lors du premier appairage.
pub fn default_device_name() -> String {
    if vtt_platform::is_steam_deck() {
        return "Steam Deck".into();
    }
    for key in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    #[cfg(windows)]
    {
        "PC Windows".into()
    }
    #[cfg(target_os = "linux")]
    {
        "PC Linux".into()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        "Ordinateur".into()
    }
}

/// Encode une chaîne (URI `vttsync://v1?...`) en matrice QR carrée (correction M).
pub fn qr_matrix(data: &str) -> Result<QrMatrix, String> {
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::M)
        .map_err(|e| format!("génération du QR code impossible : {e}"))?;
    let size = code.width();
    let colors = code.to_colors();
    let mut modules = String::with_capacity(colors.len());
    for color in colors {
        modules.push(match color {
            Color::Dark => '1',
            Color::Light => '0',
        });
    }
    Ok(QrMatrix { size, modules })
}

fn offer_to_view(offer: &PairingOffer) -> Result<PairingOfferView, String> {
    let uri = offer.to_uri();
    let qr = qr_matrix(&uri)?;
    Ok(PairingOfferView {
        uri,
        expires_at_ms: offer.expires_at_ms,
        fingerprint: offer.key.fingerprint(),
        addrs: offer.addrs.clone(),
        qr,
    })
}

fn peer_to_view(peer: &Peer, online_addr: Option<String>) -> PeerView {
    PeerView {
        device_id: peer.device_id.as_str().to_owned(),
        device_name: peer.device_name.clone(),
        fingerprint: peer.fingerprint(),
        paired_at_ms: peer.paired_at_ms,
        last_sync_ms: peer.last_sync_ms,
        last_addr: peer.last_addr.clone(),
        online_addr,
        sync_settings: peer.sync_settings,
        sync_meetings: peer.sync_meetings,
    }
}

/// Croise les appareils appairés et les appareils découverts par mDNS afin
/// d'associer les adresses en ligne et de marquer les pairs déjà connus.
fn build_peer_views(
    peers: &[Peer],
    discovered: &[DiscoveredPeer],
) -> (Vec<PeerView>, Vec<DiscoveredPeerView>) {
    let online: HashMap<&str, String> = discovered
        .iter()
        .map(|d| (d.device_id.as_str(), d.addr.to_string()))
        .collect();
    let peer_views: Vec<PeerView> = peers
        .iter()
        .map(|p| peer_to_view(p, online.get(p.device_id.as_str()).cloned()))
        .collect();

    let discovered_views: Vec<DiscoveredPeerView> = discovered
        .iter()
        .map(|d| DiscoveredPeerView {
            device_id: d.device_id.as_str().to_owned(),
            device_name: d.device_name.clone(),
            addr: d.addr.to_string(),
            already_paired: peers.iter().any(|p| p.device_id == d.device_id),
        })
        .collect();

    (peer_views, discovered_views)
}

/// Résume une synchronisation réussie en une phrase claire pour l'interface.
pub fn format_summary_message(summary: &SyncSummary) -> String {
    let mut parts = Vec::new();
    if summary.config_changed {
        parts.push("réglages fusionnés".to_owned());
    }
    if summary.usage_changed {
        parts.push("statistiques mises à jour".to_owned());
    }
    if summary.meetings_received > 0 {
        parts.push(format!(
            "{} réunion{} reçue{}",
            summary.meetings_received,
            if summary.meetings_received > 1 {
                "s"
            } else {
                ""
            },
            if summary.meetings_received > 1 {
                "s"
            } else {
                ""
            },
        ));
    }
    if summary.meetings_sent > 0 {
        parts.push(format!(
            "{} réunion{} envoyée{}",
            summary.meetings_sent,
            if summary.meetings_sent > 1 { "s" } else { "" },
            if summary.meetings_sent > 1 { "s" } else { "" },
        ));
    }
    if parts.is_empty() {
        format!("{} : déjà à jour.", summary.peer_name)
    } else {
        format!("{} : {}.", summary.peer_name, parts.join(", "))
    }
}

fn pending_to_view(
    pending: &PendingDirectPairing,
    local_approved: bool,
) -> PendingDirectPairingView {
    PendingDirectPairingView {
        session_id: pending.session_id.clone(),
        remote_device_id: pending.remote_device_id.as_str().to_owned(),
        remote_device_name: pending.remote_device_name.clone(),
        code: pending.code.clone(),
        formatted_code: pending.formatted_code(),
        expires_at_ms: pending.expires_at_ms,
        incoming: pending.incoming,
        local_approved,
    }
}

// --- Commandes Tauri ---

#[tauri::command]
pub fn sync_view(sync: State<'_, SyncState>) -> Result<SyncView, String> {
    sync.view()
}

#[tauri::command]
pub fn sync_set_listening(sync: State<'_, SyncState>, enabled: bool) -> Result<SyncView, String> {
    if enabled {
        sync.start_listening()?;
    } else {
        sync.stop_listening();
    }
    sync.view()
}

#[tauri::command]
pub fn sync_set_direct_pairing_enabled(
    sync: State<'_, SyncState>,
    enabled: bool,
) -> Result<SyncView, String> {
    sync.set_direct_pairing_enabled(enabled)
}

#[tauri::command(async)]
pub fn sync_start_direct_pairing(
    sync: State<'_, SyncState>,
    device_id: Option<String>,
    addr: String,
) -> Result<PendingDirectPairingView, String> {
    sync.inner()
        .start_direct_pairing(device_id.as_deref(), &addr)
}

#[tauri::command]
pub fn sync_respond_direct_pairing(
    sync: State<'_, SyncState>,
    session_id: String,
    approve: bool,
) -> Result<SyncView, String> {
    sync.respond_direct_pairing(&session_id, approve)
}

#[tauri::command]
pub fn sync_rename_device(sync: State<'_, SyncState>, name: String) -> Result<SyncView, String> {
    sync.rename_device(&name)
}

#[tauri::command]
pub fn sync_open_pairing(sync: State<'_, SyncState>) -> Result<PairingOfferView, String> {
    sync.open_pairing()
}

#[tauri::command]
pub fn sync_cancel_pairing(sync: State<'_, SyncState>) -> Result<SyncView, String> {
    sync.cancel_pairing();
    sync.view()
}

#[tauri::command]
pub fn sync_refresh_discovery(sync: State<'_, SyncState>) -> Result<SyncView, String> {
    sync.refresh_discovery()
}

#[tauri::command(async)]
pub fn sync_pair_uri(sync: State<'_, SyncState>, uri: String) -> Result<PeerView, String> {
    sync.pair_uri(&uri)
}

#[tauri::command(async)]
pub fn sync_now(
    sync: State<'_, SyncState>,
    device_id: String,
    addr_override: Option<String>,
) -> Result<SyncSummaryView, String> {
    sync.sync_now(&device_id, addr_override.as_deref())
}

#[tauri::command]
pub fn sync_set_peer_prefs(
    sync: State<'_, SyncState>,
    device_id: String,
    sync_settings: bool,
    sync_meetings: bool,
) -> Result<bool, String> {
    sync.set_peer_prefs(&device_id, sync_settings, sync_meetings)
}

#[tauri::command]
pub fn sync_remove_peer(sync: State<'_, SyncState>, device_id: String) -> Result<bool, String> {
    sync.remove_peer(&device_id)
}

#[cfg(test)]
mod tests {
    use vtt_core::sync::{LocalIdentity, PROTOCOL_VERSION, SyncKey};

    use super::*;

    #[test]
    fn pending_to_view_formats_six_digit_code_and_tracks_approval() {
        let id = DeviceId::generate().unwrap();
        let pending = PendingDirectPairing {
            session_id: "sess-1".into(),
            remote_device_id: id.clone(),
            remote_device_name: "Xiaomi 17T Pro".into(),
            code: "482917".into(),
            expires_at_ms: 9_999,
            incoming: true,
        };
        let view = pending_to_view(&pending, true);
        assert_eq!(view.session_id, "sess-1");
        assert_eq!(view.remote_device_id, id.as_str());
        assert_eq!(view.formatted_code, "482 917");
        assert!(view.incoming);
        assert!(view.local_approved);
    }

    #[test]
    fn qr_matrix_encodes_pairing_uri_into_valid_square() {
        let id = LocalIdentity::generate("PC Bureau").unwrap();
        let offer = PairingOffer::create(
            &id,
            vec!["192.168.1.15:41739".into()],
            1_700_000_000_000,
            Some(300_000),
        )
        .unwrap();
        let matrix = qr_matrix(&offer.to_uri()).unwrap();
        assert!(matrix.size >= 29);
        assert_eq!(matrix.modules.len(), matrix.size * matrix.size);
        assert!(matrix.modules.bytes().all(|b| b == b'0' || b == b'1'));
        assert!(matrix.modules.contains('1'));
        assert!(matrix.modules.contains('0'));
    }

    #[test]
    fn build_peer_views_attaches_online_addr_and_marks_already_paired() {
        let phone_id = DeviceId::generate().unwrap();
        let unknown_id = DeviceId::generate().unwrap();
        let peer = Peer {
            device_id: phone_id.clone(),
            device_name: "Xiaomi 17T Pro".into(),
            key: SyncKey::generate().unwrap(),
            paired_at_ms: 1_000,
            last_sync_ms: Some(2_000),
            last_addr: Some("192.168.1.20:41739".into()),
            sync_settings: true,
            sync_meetings: false,
        };
        let discovered = vec![
            DiscoveredPeer {
                device_id: phone_id.clone(),
                device_name: "Xiaomi 17T Pro".into(),
                addr: "192.168.1.44:41739".parse().unwrap(),
                protocol: PROTOCOL_VERSION,
                last_seen_ms: 3_000,
                ttl_secs: 120,
            },
            DiscoveredPeer {
                device_id: unknown_id,
                device_name: "Steam Deck".into(),
                addr: "192.168.1.50:41739".parse().unwrap(),
                protocol: PROTOCOL_VERSION,
                last_seen_ms: 3_000,
                ttl_secs: 120,
            },
        ];

        let (peers, seen) = build_peer_views(&[peer], &discovered);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].online_addr.as_deref(), Some("192.168.1.44:41739"));
        assert!(!peers[0].sync_meetings);

        assert_eq!(seen.len(), 2);
        assert!(seen[0].already_paired);
        assert!(!seen[1].already_paired);
    }

    #[test]
    fn format_summary_message_handles_all_cases() {
        let id = DeviceId::generate().unwrap();
        let base = SyncSummary {
            peer_id: id,
            peer_name: "Android".into(),
            config_changed: false,
            usage_changed: false,
            merge_report: None,
            meetings_sent: 0,
            meetings_received: 0,
            at_ms: 0,
        };
        assert_eq!(format_summary_message(&base), "Android : déjà à jour.");

        let updated = SyncSummary {
            config_changed: true,
            usage_changed: true,
            meetings_received: 2,
            meetings_sent: 1,
            ..base
        };
        assert_eq!(
            format_summary_message(&updated),
            "Android : réglages fusionnés, statistiques mises à jour, 2 réunions reçues, 1 réunion envoyée."
        );
    }
}
