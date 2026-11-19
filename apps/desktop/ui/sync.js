// Section Réglages → Synchronisation : écoute Wi-Fi locale, appairage par
// QR code (ou lien vttsync://), découverte mDNS, liste des appareils appairés
// et synchronisation en un clic (avec secours manuel IP:port).

const SVG_NS = "http://www.w3.org/2000/svg";
let currentSyncView = null;
let currentOfferUri = "";
let syncExpiryTimer = null;

function showSyncResult(text, isError = false) {
  const box = $("sync-result");
  if (!box) return;
  box.hidden = !text;
  box.textContent = text;
  box.classList.toggle("error", Boolean(isError));
}

/** Construit un élément `<svg>` net à partir de la matrice QR (`size`, `modules`). */
function buildQrSvg(qr) {
  const quiet = 4;
  const total = qr.size + quiet * 2;
  const make = (tag) =>
    typeof document.createElementNS === "function"
      ? document.createElementNS(SVG_NS, tag)
      : document.createElement(tag);

  const svg = make("svg");
  svg.setAttribute("viewBox", `0 0 ${total} ${total}`);
  svg.setAttribute("width", "192");
  svg.setAttribute("height", "192");
  svg.setAttribute("shape-rendering", "crispEdges");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", "QR code d'appairage VoiceToText");

  const bg = make("rect");
  bg.setAttribute("width", String(total));
  bg.setAttribute("height", String(total));
  bg.setAttribute("fill", "#ffffff");

  let d = "";
  for (let y = 0; y < qr.size; y++) {
    for (let x = 0; x < qr.size; x++) {
      if (qr.modules[y * qr.size + x] === "1") {
        d += `M${x + quiet} ${y + quiet}h1v1h-1z`;
      }
    }
  }
  const path = make("path");
  path.setAttribute("d", d);
  path.setAttribute("fill", "#111111");

  svg.append(bg, path);
  return svg;
}

function formatSyncDate(ms) {
  if (!ms) return "jamais";
  try {
    return new Intl.DateTimeFormat("fr-FR", {
      dateStyle: "short",
      timeStyle: "short",
    }).format(new Date(ms));
  } catch {
    return new Date(ms).toLocaleString();
  }
}

function updateQrExpiryText(expiresAtMs) {
  const el = $("sync-qr-expiry");
  if (!el) return;
  const remainingMs = expiresAtMs - Date.now();
  if (remainingMs <= 0) {
    el.textContent = "QR code expiré : cliquez sur « Afficher un QR code » pour en générer un nouveau.";
    stopQrExpiryTimer();
    return;
  }
  const sec = Math.ceil(remainingMs / 1000);
  const min = Math.floor(sec / 60);
  const rem = String(sec % 60).padStart(2, "0");
  el.textContent = `Expire dans ${min}:${rem} (usage unique).`;
}

function stopQrExpiryTimer() {
  if (syncExpiryTimer) {
    clearInterval(syncExpiryTimer);
    syncExpiryTimer = null;
  }
}

function renderActiveOffer(offer) {
  const box = $("sync-qr-box");
  const cancelBtn = $("sync-cancel-qr");
  const openBtn = $("sync-open-qr");
  stopQrExpiryTimer();

  if (!offer) {
    currentOfferUri = "";
    box.hidden = true;
    cancelBtn.hidden = true;
    openBtn.textContent = "Afficher un QR code d'appairage";
    $("sync-qr-svg").replaceChildren();
    return;
  }

  currentOfferUri = offer.uri;
  $("sync-qr-svg").replaceChildren(buildQrSvg(offer.qr));
  $("sync-qr-fingerprint").textContent = offer.fingerprint;
  $("sync-qr-addrs").textContent = offer.addrs.length
    ? `Adresses locales : ${offer.addrs.join(", ")}`
    : "";
  updateQrExpiryText(offer.expires_at_ms);
  syncExpiryTimer = setInterval(() => updateQrExpiryText(offer.expires_at_ms), 1000);

  box.hidden = false;
  cancelBtn.hidden = false;
  openBtn.textContent = "Régénérer le QR code";
}

function peerCard(peer) {
  const card = document.createElement("section");
  card.className = "card sync-peer-card";
  card.dataset.deviceId = peer.device_id;

  const header = document.createElement("div");
  header.className = "card-header";

  const titleWrap = document.createElement("div");
  titleWrap.className = "sync-peer-title";
  const name = document.createElement("strong");
  name.textContent = peer.device_name;
  const pill = document.createElement("span");
  pill.className = `sync-pill ${peer.online_addr ? "online" : "offline"}`;
  pill.textContent = peer.online_addr ? "En ligne" : "Hors ligne";
  titleWrap.append(name, pill);

  const syncBtn = document.createElement("button");
  syncBtn.type = "button";
  syncBtn.className = "primary sync-now-btn";
  syncBtn.textContent = "Synchroniser";

  const removeBtn = document.createElement("button");
  removeBtn.type = "button";
  removeBtn.className = "danger sync-remove-btn";
  removeBtn.textContent = "Oublier";

  header.append(titleWrap, syncBtn, removeBtn);

  const meta = document.createElement("p");
  meta.className = "hint sync-peer-meta";
  const addrLabel = peer.online_addr
    ? `Détecté sur ${peer.online_addr}`
    : peer.last_addr
      ? `Dernière adresse : ${peer.last_addr}`
      : "Adresse inconnue";
  meta.textContent = `${addrLabel} · Empreinte ${peer.fingerprint} · Dernière synchro : ${formatSyncDate(peer.last_sync_ms)}`;

  const prefsRow = document.createElement("div");
  prefsRow.className = "row sync-prefs-row";

  const settingsLabel = document.createElement("label");
  settingsLabel.className = "inline";
  const settingsCb = document.createElement("input");
  settingsCb.type = "checkbox";
  settingsCb.className = "sync-pref-settings";
  settingsCb.checked = Boolean(peer.sync_settings);
  const settingsText = document.createElement("span");
  settingsText.textContent = "Réglages (dictionnaire, snippets, styles, prompts, profils)";
  settingsLabel.append(settingsCb, settingsText);

  const meetingsLabel = document.createElement("label");
  meetingsLabel.className = "inline";
  const meetingsCb = document.createElement("input");
  meetingsCb.type = "checkbox";
  meetingsCb.className = "sync-pref-meetings";
  meetingsCb.checked = Boolean(peer.sync_meetings);
  const meetingsText = document.createElement("span");
  meetingsText.textContent = "Réunions";
  meetingsLabel.append(meetingsCb, meetingsText);

  prefsRow.append(settingsLabel, meetingsLabel);

  // Secours manuel IP:port si mDNS est bloqué ou l'adresse DHCP a changé.
  const fallback = document.createElement("details");
  fallback.className = "sync-fallback";
  const fallbackSummary = document.createElement("summary");
  fallbackSummary.textContent = "Adresse manuelle (secours IP:port)";
  const fallbackRow = document.createElement("div");
  fallbackRow.className = "row";
  const addrInput = document.createElement("input");
  addrInput.type = "text";
  addrInput.className = "sync-addr-override";
  addrInput.placeholder = peer.online_addr || peer.last_addr || "192.168.1.25:41739";
  addrInput.spellcheck = false;
  fallbackRow.append(addrInput);
  fallback.append(fallbackSummary, fallbackRow);

  const updatePrefs = async () => {
    try {
      await invoke("sync_set_peer_prefs", {
        deviceId: peer.device_id,
        syncSettings: settingsCb.checked,
        syncMeetings: meetingsCb.checked,
      });
    } catch (e) {
      showSyncResult(String(e), true);
      await loadSyncView();
    }
  };
  settingsCb.addEventListener("change", updatePrefs);
  meetingsCb.addEventListener("change", updatePrefs);

  syncBtn.addEventListener("click", async () => {
    syncBtn.disabled = true;
    syncBtn.textContent = "Synchronisation…";
    showSyncResult(`Synchronisation avec ${peer.device_name}…`);
    try {
      const override = addrInput.value.trim() || null;
      const summary = await invoke("sync_now", {
        deviceId: peer.device_id,
        addrOverride: override,
      });
      showSyncResult(`✓ ${summary.message}`);
      await loadSyncView();
    } catch (e) {
      showSyncResult(String(e), true);
    } finally {
      syncBtn.disabled = false;
      syncBtn.textContent = "Synchroniser";
    }
  });

  // Confirmation en deux clics pour éviter d'oublier un appareil par erreur.
  let confirmTimer = null;
  removeBtn.addEventListener("click", async () => {
    if (!confirmTimer) {
      removeBtn.textContent = "Confirmer ?";
      confirmTimer = setTimeout(() => {
        removeBtn.textContent = "Oublier";
        confirmTimer = null;
      }, 4000);
      return;
    }
    clearTimeout(confirmTimer);
    confirmTimer = null;
    removeBtn.disabled = true;
    try {
      await invoke("sync_remove_peer", { deviceId: peer.device_id });
      showSyncResult(`Appareil « ${peer.device_name} » oublié (clé révoquée).`);
      await loadSyncView();
    } catch (e) {
      showSyncResult(String(e), true);
      removeBtn.disabled = false;
      removeBtn.textContent = "Oublier";
    }
  });

  card.append(header, meta, prefsRow, fallback);
  return card;
}

function discoveredItem(peer) {
  const li = document.createElement("li");
  li.className = "sync-discovered-item";
  const strong = document.createElement("strong");
  strong.textContent = peer.device_name;
  const meta = document.createElement("span");
  meta.className = "muted small";
  meta.textContent = ` ${peer.addr} — ${peer.already_paired ? "Déjà appairé" : "Non appairé (scannez son QR code)"}`;
  li.append(strong, meta);
  return li;
}

function renderSyncView(view) {
  currentSyncView = view;
  const badge = $("sync-status-badge");
  const toggleBtn = $("sync-toggle-listen");
  const detail = $("sync-status-detail");

  badge.textContent = view.listening ? "Écoute active" : "Écoute arrêtée";
  badge.classList.toggle("active", Boolean(view.listening));
  toggleBtn.textContent = view.listening ? "Arrêter l'écoute" : "Activer l'écoute locale";
  toggleBtn.classList.toggle("primary", !view.listening);

  if (view.server_error) {
    detail.textContent = `Erreur serveur : ${view.server_error}`;
  } else if (view.listening) {
    const addrs = view.local_addrs.length
      ? view.local_addrs.join(", ")
      : `port ${view.port}`;
    detail.textContent = `Joignable sur le réseau local : ${addrs}.`;
  } else {
    detail.textContent =
      "Aucun port réseau n'est ouvert tant que l'écoute locale est arrêtée.";
  }

  const nameInput = $("sync-device-name");
  if (document.activeElement !== nameInput) {
    nameInput.value = view.device_name;
  }

  renderActiveOffer(view.active_offer);

  const peersList = $("sync-peers-list");
  peersList.replaceChildren(...view.peers.map(peerCard));
  $("sync-peers-empty").hidden = view.peers.length > 0;

  const mdnsNote = $("sync-mdns-note");
  if (!view.listening) {
    mdnsNote.textContent = "Activez l'écoute locale pour découvrir les appareils sur le Wi-Fi.";
  } else if (view.mdns_active) {
    mdnsNote.textContent = "Découverte automatique Wi-Fi (_voicetotext._tcp.local.) active.";
  } else {
    mdnsNote.textContent = view.mdns_error
      ? `Découverte automatique indisponible (${view.mdns_error}). Utilisez le QR code ou l'adresse IP:port.`
      : "Découverte automatique inactive.";
  }

  const discList = $("sync-discovered-list");
  discList.replaceChildren(...view.discovered.map(discoveredItem));
  $("sync-discovered-empty").hidden = view.discovered.length > 0;

  if (view.last_activity && $("sync-result").hidden) {
    showSyncResult(view.last_activity.message, view.last_activity.is_error);
  }
}

async function loadSyncView() {
  if (!$("sync-status-badge")) return;
  try {
    const view = await invoke("sync_view");
    renderSyncView(view);
  } catch (e) {
    showSyncResult(String(e), true);
  }
}

function initSyncUI() {
  if (!$("sync-status-badge")) return;

  $("sync-toggle-listen").addEventListener("click", async () => {
    const btn = $("sync-toggle-listen");
    const want = !currentSyncView?.listening;
    btn.disabled = true;
    try {
      const view = await invoke("sync_set_listening", { enabled: want });
      renderSyncView(view);
      showSyncResult(want ? "✓ Écoute Wi-Fi locale activée." : "Écoute Wi-Fi locale arrêtée.");
    } catch (e) {
      showSyncResult(String(e), true);
    } finally {
      btn.disabled = false;
    }
  });

  $("sync-refresh").addEventListener("click", async () => {
    const btn = $("sync-refresh");
    btn.disabled = true;
    try {
      const view = await invoke("sync_refresh_discovery");
      renderSyncView(view);
    } catch (e) {
      showSyncResult(String(e), true);
    } finally {
      btn.disabled = false;
    }
  });

  $("sync-rename-btn").addEventListener("click", async () => {
    const name = $("sync-device-name").value.trim();
    if (!name) return;
    try {
      const view = await invoke("sync_rename_device", { name });
      renderSyncView(view);
      showSyncResult(`✓ Appareil renommé en « ${view.device_name} ».`);
    } catch (e) {
      showSyncResult(String(e), true);
    }
  });

  $("sync-open-qr").addEventListener("click", async () => {
    const btn = $("sync-open-qr");
    btn.disabled = true;
    try {
      const offer = await invoke("sync_open_pairing");
      const view = await invoke("sync_view");
      renderSyncView({ ...view, active_offer: offer });
      showSyncResult("QR code d'appairage prêt (valide 5 minutes, usage unique).");
    } catch (e) {
      showSyncResult(String(e), true);
    } finally {
      btn.disabled = false;
    }
  });

  $("sync-cancel-qr").addEventListener("click", async () => {
    try {
      const view = await invoke("sync_cancel_pairing");
      renderSyncView(view);
      showSyncResult("Offre d'appairage fermée.");
    } catch (e) {
      showSyncResult(String(e), true);
    }
  });

  $("sync-copy-uri").addEventListener("click", async () => {
    if (!currentOfferUri) return;
    try {
      await navigator.clipboard.writeText(currentOfferUri);
      const btn = $("sync-copy-uri");
      const prev = btn.textContent;
      btn.textContent = "Copié ✓";
      setTimeout(() => (btn.textContent = prev), 1500);
    } catch (e) {
      showSyncResult(String(e), true);
    }
  });

  $("sync-pair-btn").addEventListener("click", async () => {
    const input = $("sync-pair-uri");
    const uri = input.value.trim();
    if (!uri) {
      showSyncResult("Collez d'abord un lien d'appairage (vttsync://v1?...).", true);
      return;
    }
    const btn = $("sync-pair-btn");
    btn.disabled = true;
    showSyncResult("Appairage en cours…");
    try {
      const peer = await invoke("sync_pair_uri", { uri });
      input.value = "";
      showSyncResult(`✓ Appareil « ${peer.device_name} » appairé.`);
      await loadSyncView();
    } catch (e) {
      showSyncResult(String(e), true);
    } finally {
      btn.disabled = false;
    }
  });

  window.__TAURI__?.event?.listen("sync-changed", () => {
    if (isShown("reglages")) loadSyncView();
  });
}

initSyncUI();
