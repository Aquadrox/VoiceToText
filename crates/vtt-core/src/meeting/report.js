// Rapport VoiceToText : sommaire, recherche (sans casse ni accents),
// filtres, renvois vers la transcription. Dans l'application, l'objet
// `VTT` fourni par celle-ci enregistre les actions cochées et lit l'audio.
(function () {
  "use strict";
  var $ = function (id) { return document.getElementById(id); };
  var doc = $("doc");
  var app = document.documentElement.dataset.mode === "app" && typeof window.VTT === "object";

  // ---- Sommaire ----
  var toggle = $("toc-toggle");
  function showToc(open) {
    document.body.classList.toggle("toc-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  }
  toggle.addEventListener("click", function () {
    showToc(!document.body.classList.contains("toc-open"));
  });
  $("toc").addEventListener("click", function (e) {
    var link = e.target.closest("a[href^='#']");
    if (!link) return;
    e.preventDefault();
    showToc(false);
    var target = document.getElementById(link.getAttribute("href").slice(1));
    if (target) target.scrollIntoView({ block: "start" });
  });

  // ---- Recherche ----
  // Minuscule sans accent, caractère pour caractère : les positions restent
  // celles du texte affiché.
  function fold(text) {
    var out = "";
    for (var i = 0; i < text.length; i++) {
      var c = text[i];
      var f = c === "œ" || c === "Œ" ? "o" : c === "æ" || c === "Æ" ? "a" : c.normalize("NFD")[0].toLowerCase();
      out += f.length === 1 ? f : c;
    }
    return out;
  }
  var input = $("q"), count = $("count"), prev = $("prev"), next = $("next");
  var hits = [], current = -1, timer = 0;

  function clearHits() {
    hits.forEach(function (mark) {
      var parent = mark.parentNode;
      if (!parent) return;
      parent.replaceChild(document.createTextNode(mark.textContent), mark);
      parent.normalize();
    });
    hits = [];
    current = -1;
    document.querySelectorAll("#toc .hits").forEach(function (h) { h.textContent = ""; });
  }

  function hidden(node) {
    return node.parentElement.closest(".f-hide, script, style, button") !== null;
  }

  function search() {
    clearHits();
    var query = fold(input.value.trim().replace(/\s+/g, " "));
    if (query.length < 2) {
      count.textContent = "";
      prev.disabled = next.disabled = true;
      return;
    }
    var walker = document.createTreeWalker(doc, NodeFilter.SHOW_TEXT);
    var nodes = [];
    while (walker.nextNode()) {
      if (!hidden(walker.currentNode)) nodes.push(walker.currentNode);
    }
    nodes.forEach(function (node) {
      var folded = fold(node.data);
      var at = folded.indexOf(query);
      if (at < 0) return;
      var fragment = document.createDocumentFragment();
      var last = 0;
      while (at >= 0) {
        fragment.appendChild(document.createTextNode(node.data.slice(last, at)));
        var mark = document.createElement("mark");
        mark.className = "hit";
        mark.textContent = node.data.slice(at, at + query.length);
        fragment.appendChild(mark);
        hits.push(mark);
        last = at + query.length;
        at = folded.indexOf(query, last);
      }
      fragment.appendChild(document.createTextNode(node.data.slice(last)));
      node.parentNode.replaceChild(fragment, node);
    });
    // Nombre de résultats par partie, dans le sommaire.
    var perSection = {};
    hits.forEach(function (mark) {
      var section = mark.closest("[data-section]");
      if (section) perSection[section.id] = (perSection[section.id] || 0) + 1;
    });
    Object.keys(perSection).forEach(function (id) {
      var link = document.querySelector("#toc a[href='#" + id + "']");
      var badge = link && link.parentElement.querySelector(":scope > .hits");
      if (badge) badge.textContent = perSection[id];
    });
    prev.disabled = next.disabled = hits.length === 0;
    if (hits.length === 0) {
      count.textContent = "0 résultat";
    } else {
      go(0);
    }
  }

  function go(index) {
    if (hits.length === 0) return;
    if (current >= 0 && hits[current]) hits[current].classList.remove("current");
    current = (index + hits.length) % hits.length;
    var mark = hits[current];
    mark.classList.add("current");
    var details = mark.closest("details");
    if (details) details.open = true;
    mark.scrollIntoView({ block: "center" });
    count.textContent = current + 1 + " / " + hits.length;
  }

  input.addEventListener("input", function () {
    clearTimeout(timer);
    timer = setTimeout(search, 250);
  });
  input.addEventListener("keydown", function (e) {
    if (e.key === "Enter") {
      e.preventDefault();
      clearTimeout(timer);
      if (hits.length === 0) search();
      else go(current + (e.shiftKey ? -1 : 1));
    } else if (e.key === "Escape") {
      input.value = "";
      search();
    }
  });
  prev.addEventListener("click", function () { go(current - 1); });
  next.addEventListener("click", function () { go(current + 1); });

  // ---- Filtres ----
  var filter = "all";
  var who = $("who");
  function applyFilters() {
    var filtering = filter !== "all";
    document.body.classList.toggle("filtering", filtering);
    doc.querySelectorAll(".item").forEach(function (item) {
      item.classList.toggle("f-hide", filtering && !item.classList.contains("k-" + filter));
    });
    var speaker = who ? who.value : "";
    doc.querySelectorAll(".reply").forEach(function (reply) {
      reply.classList.toggle("f-hide", speaker !== "" && reply.dataset.who !== speaker);
    });
    if (speaker !== "") doc.querySelectorAll("details.transcript").forEach(function (d) { d.open = true; });
    // Titres sans rien d'affiché dessous : cachés aussi.
    doc.querySelectorAll(".group, ul.points, ol.actions, dl.qa, section[data-section], article.chapter, #plan-action").forEach(function (box) {
      box.classList.remove("f-hide");
    });
    if (filtering) {
      doc.querySelectorAll("ul.points, ol.actions, dl.qa, .group").forEach(function (box) {
        box.classList.toggle("f-hide", !box.querySelector(".item:not(.f-hide)"));
      });
      doc.querySelectorAll("section[data-section], article.chapter").forEach(function (box) {
        box.classList.toggle("f-hide", !box.querySelector(".item:not(.f-hide)"));
      });
    }
    if (input.value.trim().length >= 2) search();
  }
  var note = $("filter-note");
  function setFilter(chip) {
    filter = chip.dataset.filter;
    document.querySelectorAll(".chip[data-filter]").forEach(function (c) {
      c.setAttribute("aria-pressed", String(c === chip));
    });
    note.hidden = filter === "all";
    note.firstChild.textContent = "Affiché : " + chip.textContent + " seulement.";
    applyFilters();
    showToc(false);
    window.scrollTo(0, 0);
  }
  document.querySelectorAll(".chip[data-filter]").forEach(function (chip) {
    chip.addEventListener("click", function () { setFilter(chip); });
  });
  $("filter-reset").addEventListener("click", function () {
    setFilter(document.querySelector(".chip[data-filter=all]"));
  });
  if (who) who.addEventListener("change", function () { applyFilters(); showToc(false); });

  // ---- Renvois vers la transcription ----
  function reveal(ms) {
    var replies = doc.querySelectorAll(".reply:not(.f-hide)");
    var target = null;
    for (var i = 0; i < replies.length; i++) {
      if (Number(replies[i].dataset.ms) <= ms) target = replies[i];
      else break;
    }
    if (!target) target = replies[0];
    if (!target) return;
    var details = target.closest("details");
    if (details) details.open = true;
    target.scrollIntoView({ block: "center" });
    target.classList.remove("flash");
    void target.offsetWidth;
    target.classList.add("flash");
  }
  doc.addEventListener("click", function (e) {
    var link = e.target.closest("a.t");
    if (link) {
      e.preventDefault();
      // Depuis la transcription : rien à chercher, on y est déjà.
      if (!link.closest(".reply")) reveal(Number(link.dataset.ms));
      return;
    }
    var play = e.target.closest("button.play");
    if (play && app) {
      if (play.classList.contains("playing")) window.VTT.stop();
      else window.VTT.play(Number(play.dataset.ms), Number(play.dataset.end));
    }
  });

  // Lecture en cours, signalée par l'application (-1 : aucune).
  window.vttPlaying = function (ms) {
    doc.querySelectorAll("button.play").forEach(function (b) {
      var on = Number(b.dataset.ms) === ms;
      b.classList.toggle("playing", on);
      b.textContent = on ? "❚❚" : "▶";
      b.setAttribute("aria-label", (on ? "Arrêter la lecture" : "Réécouter à " + b.parentElement.querySelector("a.t").textContent));
    });
  };

  // ---- Plan d'action ----
  var doneCount = $("done-count");
  doc.addEventListener("change", function (e) {
    var box = e.target;
    if (!box.matches(".action input[type=checkbox]")) return;
    box.closest(".action").classList.toggle("done", box.checked);
    if (app) window.VTT.setDone(Number(box.dataset.index), box.checked);
    if (doneCount) {
      var done = doc.querySelectorAll(".action input:checked").length;
      doneCount.textContent = done + " / " + doneCount.dataset.total + " faite(s)";
    }
  });

  // Impression (ou PDF) : transcriptions dépliées.
  window.addEventListener("beforeprint", function () {
    doc.querySelectorAll("details").forEach(function (d) { d.open = true; });
  });
})();
