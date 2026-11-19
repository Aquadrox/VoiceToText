import test from "node:test";
import assert from "node:assert/strict";
import { createEnvironment } from "./dom.mjs";

const flush = () => new Promise((resolve) => setImmediate(resolve));

function setupSync(invoke) {
  const env = createEnvironment({ invoke });
  const { el, document } = env;

  const panel = el(
    "section",
    { className: "panel" },
    el("div", { id: "sync-result", className: "result", hidden: true }),
    el(
      "div",
      { id: "sync-direct-box", className: "sync-direct-box", hidden: true },
      el("p", { id: "sync-direct-title", className: "sync-direct-title" }),
      el("div", { id: "sync-direct-code", className: "sync-direct-code" }),
      el("p", { id: "sync-direct-expiry", className: "hint" }),
      el("button", { id: "sync-direct-approve", textContent: "Appairer (OK)" }),
      el("button", { id: "sync-direct-reject", textContent: "Refuser" }),
    ),
    el("span", { id: "sync-status-badge", className: "sync-badge" }),
    el("button", { id: "sync-toggle-listen", textContent: "Activer l'écoute locale" }),
    el("button", { id: "sync-refresh", textContent: "Actualiser" }),
    el("p", { id: "sync-status-detail", className: "hint" }),
    el("input", { id: "sync-device-name", value: "" }),
    el("button", { id: "sync-rename-btn", textContent: "Renommer" }),
    el("button", { id: "sync-open-qr", textContent: "Afficher un QR code d'appairage" }),
    el("button", { id: "sync-cancel-qr", textContent: "Fermer le QR code", hidden: true }),
    el(
      "div",
      { id: "sync-qr-box", className: "sync-qr-box", hidden: true },
      el("div", { id: "sync-qr-svg", className: "sync-qr-svg" }),
      el("code", { id: "sync-qr-fingerprint" }),
      el("p", { id: "sync-qr-addrs" }),
      el("p", { id: "sync-qr-expiry" }),
      el("button", { id: "sync-copy-uri", textContent: "Copier le lien d'appairage" }),
    ),
    el("input", { id: "sync-pair-uri", value: "" }),
    el("button", { id: "sync-pair-btn", textContent: "Appairer avec ce lien" }),
    el("p", { id: "sync-peers-empty", hidden: false }),
    el("div", { id: "sync-peers-list", className: "cards" }),
    el("p", { id: "sync-mdns-note" }),
    el("p", { id: "sync-discovered-empty", hidden: false }),
    el("ul", { id: "sync-discovered-list" }),
  );
  document.body.append(panel);
  env.load("common.js", "sync.js");
  return { env, document };
}

function sampleView(overrides = {}) {
  return {
    listening: true,
    port: 41739,
    local_addrs: ["192.168.1.10:41739"],
    mdns_active: true,
    mdns_error: null,
    server_error: null,
    device_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    device_name: "PC Bureau",
    direct_pairing_enabled: true,
    active_offer: null,
    pending_direct: null,
    discovered: [
      {
        device_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        device_name: "Xiaomi 17T Pro",
        addr: "192.168.1.25:41739",
        already_paired: true,
      },
      {
        device_id: "cccccccccccccccccccccccccccccccc",
        device_name: "Steam Deck",
        addr: "192.168.1.30:41739",
        already_paired: false,
      },
    ],
    peers: [
      {
        device_id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        device_name: "Xiaomi 17T Pro",
        fingerprint: "1A2B-3C4D-5E6F-7081",
        paired_at_ms: 1_700_000_000_000,
        last_sync_ms: 1_700_000_100_000,
        last_addr: "192.168.1.25:41739",
        online_addr: "192.168.1.25:41739",
        sync_settings: true,
        sync_meetings: true,
      },
    ],
    last_activity: null,
    ...overrides,
  };
}

test("loadSyncView affiche l'état local, les pairs appairés et les appareils mDNS", async () => {
  const { env, document } = setupSync(async (cmd) => {
    if (cmd === "sync_view") return sampleView();
    return {};
  });

  await env.eval("loadSyncView")();

  assert.equal(document.getElementById("sync-status-badge").textContent, "Écoute active");
  assert.match(document.getElementById("sync-status-detail").textContent, /192\.168\.1\.10:41739/);
  assert.equal(document.getElementById("sync-device-name").value, "PC Bureau");

  const peersList = document.getElementById("sync-peers-list");
  assert.equal(peersList.children.length, 1);
  assert.equal(document.getElementById("sync-peers-empty").hidden, true);
  assert.equal(peersList.children[0].querySelector(".sync-pill").textContent, "En ligne");
  assert.match(peersList.children[0].querySelector(".sync-peer-meta").textContent, /1A2B-3C4D-5E6F-7081/);

  const discList = document.getElementById("sync-discovered-list");
  assert.equal(discList.children.length, 2);
  assert.match(discList.children[0].textContent, /Déjà appairé/);
  assert.match(discList.children[1].textContent, /Non appairé/);
});

test("l'ouverture d'un QR code génère le SVG exact, copie le lien et se ferme proprement", async () => {
  const offer = {
    uri: "vttsync://v1?id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&n=PC&a=192.168.1.10:41739&k=abc&exp=9999999999999",
    expires_at_ms: Date.now() + 300_000,
    fingerprint: "AAAA-BBBB-CCCC-DDDD",
    addrs: ["192.168.1.10:41739"],
    qr: { size: 3, modules: "101010101" },
  };
  let activeOffer = null;
  let copied = "";

  const { env, document } = setupSync(async (cmd) => {
    if (cmd === "sync_open_pairing") {
      activeOffer = offer;
      return offer;
    }
    if (cmd === "sync_cancel_pairing") {
      activeOffer = null;
      return sampleView({ active_offer: null });
    }
    if (cmd === "sync_view") return sampleView({ active_offer: activeOffer });
    return {};
  });
  env.clipboard.writeText = async (text) => {
    copied = text;
  };

  document.getElementById("sync-open-qr").dispatchEvent("click");
  await flush();
  await flush();

  const qrBox = document.getElementById("sync-qr-box");
  assert.equal(qrBox.hidden, false);
  assert.equal(document.getElementById("sync-qr-fingerprint").textContent, "AAAA-BBBB-CCCC-DDDD");

  const svg = document.getElementById("sync-qr-svg").children[0];
  assert.equal(svg.tagName, "SVG");
  assert.equal(svg.getAttribute("viewBox"), "0 0 11 11");
  const path = svg.querySelector("path");
  // 5 modules actifs dans "101010101" -> 5 commandes M...z
  assert.equal((path.getAttribute("d").match(/M/g) || []).length, 5);

  document.getElementById("sync-copy-uri").dispatchEvent("click");
  await flush();
  assert.equal(copied, offer.uri);

  document.getElementById("sync-cancel-qr").dispatchEvent("click");
  await flush();
  assert.equal(qrBox.hidden, true);
  assert.equal(document.getElementById("sync-qr-svg").children.length, 0);
});

test("appairage manuel, synchronisation avec secours IP:port et oubli confirmé en deux clics", async () => {
  const calls = [];
  let peers = sampleView().peers;

  const { env, document } = setupSync(async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "sync_view") return sampleView({ peers });
    if (cmd === "sync_pair_uri") {
      return {
        device_id: "dddddddddddddddddddddddddddddddd",
        device_name: "Steam Deck",
      };
    }
    if (cmd === "sync_now") {
      return {
        peer_id: args.deviceId,
        peer_name: "Xiaomi 17T Pro",
        config_changed: true,
        meetings_sent: 1,
        meetings_received: 2,
        message: "Xiaomi 17T Pro : réglages fusionnés, 2 réunions reçues, 1 réunion envoyée.",
        at_ms: Date.now(),
      };
    }
    if (cmd === "sync_remove_peer") {
      peers = [];
      return true;
    }
    return {};
  });

  await env.eval("loadSyncView")();

  // 1. Appairage via URI collée.
  const uriInput = document.getElementById("sync-pair-uri");
  uriInput.value = "vttsync://v1?id=dddddddddddddddddddddddddddddddd&n=Deck&a=192.168.1.30:41739&k=xyz&exp=9999999999999";
  document.getElementById("sync-pair-btn").dispatchEvent("click");
  await flush();
  await flush();
  assert.equal(uriInput.value, "");
  assert.ok(calls.some((c) => c.cmd === "sync_pair_uri"));

  // 2. Synchro avec adresse manuelle IP:port.
  const card = document.getElementById("sync-peers-list").children[0];
  const addrInput = card.querySelector(".sync-addr-override");
  addrInput.value = "192.168.1.99:41739";
  card.querySelector(".sync-now-btn").dispatchEvent("click");
  await flush();
  await flush();
  const syncCall = calls.find((c) => c.cmd === "sync_now");
  assert.equal(syncCall.args.addrOverride, "192.168.1.99:41739");
  assert.match(document.getElementById("sync-result").textContent, /2 réunions reçues/);

  // 3. Oublier un pair exige 2 clics.
  const refreshedCard = document.getElementById("sync-peers-list").children[0];
  const removeBtn = refreshedCard.querySelector(".sync-remove-btn");
  removeBtn.dispatchEvent("click");
  await flush();
  assert.equal(removeBtn.textContent, "Confirmer ?");
  assert.equal(calls.filter((c) => c.cmd === "sync_remove_peer").length, 0);

  removeBtn.dispatchEvent("click");
  await flush();
  await flush();
  assert.equal(calls.filter((c) => c.cmd === "sync_remove_peer").length, 1);
  assert.equal(document.getElementById("sync-peers-list").children.length, 0);
  assert.equal(document.getElementById("sync-peers-empty").hidden, false);
});

test("appairage direct par clic sur un appareil détecté : affiche le code 6 chiffres, confirme puis gère le refus", async () => {
  const calls = [];
  let pending = null;

  const { env, document } = setupSync(async (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "sync_view") return sampleView({ pending_direct: pending });
    if (cmd === "sync_start_direct_pairing") {
      pending = {
        session_id: "sess-direct-1",
        remote_device_id: args.deviceId,
        remote_device_name: "Steam Deck",
        code: "482917",
        formatted_code: "482 917",
        expires_at_ms: Date.now() + 60_000,
        incoming: false,
        local_approved: false,
      };
      return pending;
    }
    if (cmd === "sync_respond_direct_pairing") {
      if (args.approve && pending) {
        pending = { ...pending, local_approved: true };
      } else {
        pending = null;
      }
      return sampleView({ pending_direct: pending });
    }
    return {};
  });

  await env.eval("loadSyncView")();

  const discList = document.getElementById("sync-discovered-list");
  // L'appareil déjà appairé n'a pas de bouton d'appairage direct.
  assert.equal(discList.children[0].querySelector(".sync-direct-pair-btn"), null);
  // L'appareil non appairé propose un bouton « Appairer ».
  const directBtn = discList.children[1].querySelector(".sync-direct-pair-btn");
  assert.ok(directBtn);
  assert.equal(directBtn.textContent, "Appairer");

  // 1. Clic sur l'appareil détecté -> affiche le code à 6 chiffres.
  directBtn.dispatchEvent("click");
  await flush();
  await flush();

  const startCall = calls.find((c) => c.cmd === "sync_start_direct_pairing");
  assert.equal(startCall.args.deviceId, "cccccccccccccccccccccccccccccccc");
  assert.equal(startCall.args.addr, "192.168.1.30:41739");
  assert.equal(document.getElementById("sync-direct-box").hidden, false);
  assert.equal(document.getElementById("sync-direct-code").textContent, "482 917");
  assert.match(document.getElementById("sync-direct-title").textContent, /Steam Deck/);

  // 2. Clic sur Appairer (OK) -> envoie approve: true et passe en attente.
  const approveBtn = document.getElementById("sync-direct-approve");
  approveBtn.dispatchEvent("click");
  await flush();
  await flush();

  const approveCall = calls.find(
    (c) => c.cmd === "sync_respond_direct_pairing" && c.args.approve === true,
  );
  assert.equal(approveCall.args.sessionId, "sess-direct-1");
  assert.equal(approveCall.args.approve, true);
  assert.equal(approveBtn.disabled, true);
  assert.match(approveBtn.textContent, /En attente/);

  // 3. Clic sur Annuler -> envoie approve: false et ferme la boîte.
  document.getElementById("sync-direct-reject").dispatchEvent("click");
  await flush();
  await flush();

  const rejectCall = calls.find(
    (c) => c.cmd === "sync_respond_direct_pairing" && c.args.approve === false,
  );
  assert.equal(rejectCall.args.sessionId, "sess-direct-1");
  assert.equal(rejectCall.args.approve, false);
  assert.equal(document.getElementById("sync-direct-box").hidden, true);
});
