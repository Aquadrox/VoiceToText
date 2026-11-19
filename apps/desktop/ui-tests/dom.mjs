// Harnais DOM minimal (sans dépendance npm) pour exécuter et tester les scripts
// réels de apps/desktop/ui/ dans Node.js via node:vm.
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const UI_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../ui");

class FakeClassList {
  constructor(el) {
    this.el = el;
    this.set = new Set();
  }
  add(...names) {
    for (const n of names) if (n) this.set.add(n);
  }
  remove(...names) {
    for (const n of names) this.set.delete(n);
  }
  contains(name) {
    return this.set.has(name);
  }
  toggle(name, force) {
    if (force === undefined) {
      if (this.set.has(name)) {
        this.set.delete(name);
        return false;
      }
      this.set.add(name);
      return true;
    }
    if (force) this.set.add(name);
    else this.set.delete(name);
    return Boolean(force);
  }
  setFromString(str) {
    this.set = new Set(String(str ?? "").split(/\s+/).filter(Boolean));
  }
  toString() {
    return [...this.set].join(" ");
  }
}

function camelToKebab(str) {
  return str.replace(/[A-Z]/g, (m) => `-${m.toLowerCase()}`);
}

function kebabToCamel(str) {
  return str.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
}

/** Vérifie un sélecteur simple (tag, #id, .cls, [attr], [attr="val"]). */
function matchesSimple(el, part) {
  let rest = part.trim();
  if (!rest) return false;
  const tagMatch = rest.match(/^[a-zA-Z][a-zA-Z0-9-]*/);
  if (tagMatch) {
    if (el.tagName !== tagMatch[0].toUpperCase()) return false;
    rest = rest.slice(tagMatch[0].length);
  }
  while (rest.length > 0) {
    if (rest[0] === "#") {
      const m = rest.match(/^#([a-zA-Z0-9_-]+)/);
      if (!m || el.id !== m[1]) return false;
      rest = rest.slice(m[0].length);
    } else if (rest[0] === ".") {
      const m = rest.match(/^\.([a-zA-Z0-9_-]+)/);
      if (!m || !el.classList.contains(m[1])) return false;
      rest = rest.slice(m[0].length);
    } else if (rest[0] === "[") {
      const m = rest.match(/^\[([a-zA-Z0-9_-]+)(?:=(?:"([^"]*)"|'([^']*)'|([^\]]*)))?\]/);
      if (!m) return false;
      const attr = m[1];
      const expected = m[2] ?? m[3] ?? m[4];
      if (!el.hasAttribute(attr)) return false;
      if (expected !== undefined && el.getAttribute(attr) !== expected) return false;
      rest = rest.slice(m[0].length);
    } else {
      return false;
    }
  }
  return true;
}

/** Vérifie un sélecteur avec combinateurs descendants (ex. « nav.main-nav a »). */
function matchesSingle(el, selector) {
  const parts = selector.trim().split(/\s+/);
  if (!matchesSimple(el, parts[parts.length - 1])) return false;
  let current = el.parentElement;
  for (let i = parts.length - 2; i >= 0; i--) {
    while (current && !matchesSimple(current, parts[i])) {
      current = current.parentElement;
    }
    if (!current) return false;
    current = current.parentElement;
  }
  return true;
}

function matchesSelector(el, selector) {
  return selector.split(",").some((s) => matchesSingle(el, s));
}

export class FakeElement {
  constructor(tagName, ownerDocument, props = {}) {
    this.tagName = tagName.toUpperCase();
    this.ownerDocument = ownerDocument;
    this.parentElement = null;
    this.children = [];
    this.attributes = new Map();
    this.listeners = new Map();
    this.classList = new FakeClassList(this);
    this.style = {};
    this.hidden = false;
    this.disabled = false;
    this.open = false;
    this.value = "";
    this.checked = false;
    this.type = "";
    this.name = "";
    this.title = "";
    this.hash = "";
    this.focused = false;
    this._textContent = "";
    this.content = null;

    this.dataset = new Proxy(
      {},
      {
        get: (_, prop) => {
          if (typeof prop !== "string") return undefined;
          return this.attributes.get(`data-${camelToKebab(prop)}`);
        },
        set: (_, prop, val) => {
          if (typeof prop === "string") {
            this.setAttribute(`data-${camelToKebab(prop)}`, String(val));
          }
          return true;
        },
        has: (_, prop) => typeof prop === "string" && this.attributes.has(`data-${camelToKebab(prop)}`),
        ownKeys: () =>
          [...this.attributes.keys()]
            .filter((k) => k.startsWith("data-"))
            .map((k) => kebabToCamel(k.slice(5))),
      },
    );

    if (props.id) this.id = props.id;
    if (props.className) this.className = props.className;
    if (props.attrs) {
      for (const [k, v] of Object.entries(props.attrs)) this.setAttribute(k, v);
    }
    for (const key of ["type", "name", "value", "checked", "hidden", "disabled", "hash", "title", "textContent"]) {
      if (props[key] !== undefined) this[key] = props[key];
    }
  }

  get id() {
    return this.attributes.get("id") ?? "";
  }
  set id(v) {
    this.setAttribute("id", String(v));
  }

  get className() {
    return this.classList.toString();
  }
  set className(v) {
    this.classList.setFromString(v);
  }

  get textContent() {
    if (this.children.length === 0) return this._textContent;
    return this.children.map((c) => c.textContent).join("");
  }
  set textContent(v) {
    this.children = [];
    this._textContent = String(v ?? "");
  }

  get firstChild() {
    return this.children[0] ?? null;
  }
  get firstElementChild() {
    return this.children[0] ?? null;
  }
  get lastElementChild() {
    return this.children[this.children.length - 1] ?? null;
  }

  setAttribute(name, value) {
    const val = String(value);
    this.attributes.set(name, val);
    if (name === "class") this.classList.setFromString(val);
    if (name === "value") this.value = val;
    if (name === "name") this.name = val;
    if (name === "type") this.type = val;
    if (name === "href" && val.startsWith("#")) this.hash = val;
    this.ownerDocument?._notifyMutation(this, name);
  }

  getAttribute(name) {
    if (name === "value" && !this.attributes.has("value")) return this.value;
    if (name === "name" && !this.attributes.has("name")) return this.name;
    if (name === "type" && !this.attributes.has("type")) return this.type;
    return this.attributes.get(name) ?? null;
  }

  hasAttribute(name) {
    if (name === "value" && this.value !== "") return true;
    if (name === "name" && this.name !== "") return true;
    if (name === "type" && this.type !== "") return true;
    return this.attributes.has(name);
  }

  removeAttribute(name) {
    this.attributes.delete(name);
    this.ownerDocument?._notifyMutation(this, name);
  }

  append(...nodes) {
    for (const node of nodes) {
      if (node == null) continue;
      if (node.parentElement) node.remove();
      node.parentElement = this;
      this.children.push(node);
    }
  }

  replaceChildren(...nodes) {
    for (const child of this.children) child.parentElement = null;
    this.children = [];
    this._textContent = "";
    this.append(...nodes);
  }

  remove() {
    if (!this.parentElement) return;
    const idx = this.parentElement.children.indexOf(this);
    if (idx >= 0) this.parentElement.children.splice(idx, 1);
    this.parentElement = null;
  }

  cloneNode(deep = false) {
    const copy = new FakeElement(this.tagName, this.ownerDocument);
    for (const [k, v] of this.attributes) copy.attributes.set(k, v);
    copy.classList.setFromString(this.className);
    copy.hidden = this.hidden;
    copy.disabled = this.disabled;
    copy.open = this.open;
    copy.value = this.value;
    copy.checked = this.checked;
    copy.type = this.type;
    copy.name = this.name;
    copy.title = this.title;
    copy.hash = this.hash;
    copy._textContent = this._textContent;
    if (deep) {
      for (const child of this.children) copy.append(child.cloneNode(true));
    }
    return copy;
  }

  matches(selector) {
    return matchesSelector(this, selector);
  }

  closest(selector) {
    let cur = this;
    while (cur) {
      if (cur.matches(selector)) return cur;
      cur = cur.parentElement;
    }
    return null;
  }

  querySelectorAll(selector) {
    const found = [];
    const walk = (node) => {
      for (const child of node.children) {
        if (child.matches(selector)) found.push(child);
        walk(child);
      }
    };
    walk(this);
    return found;
  }

  querySelector(selector) {
    return this.querySelectorAll(selector)[0] ?? null;
  }

  addEventListener(type, fn) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(fn);
  }

  dispatchEvent(event) {
    const ev = {
      type: typeof event === "string" ? event : event.type,
      target: this,
      defaultPrevented: false,
      preventDefault() {
        this.defaultPrevented = true;
      },
      ...(typeof event === "object" ? event : {}),
    };
    let cur = this;
    while (cur) {
      for (const fn of cur.listeners.get(ev.type) ?? []) fn(ev);
      cur = cur.parentElement;
    }
    return !ev.defaultPrevented;
  }

  click() {
    this.dispatchEvent("click");
  }

  focus() {
    this.focused = true;
  }

  scrollIntoView() {}

  requestSubmit() {
    this.dispatchEvent({ type: "submit" });
  }
}

export class FakeDocument {
  constructor() {
    this.hidden = false;
    this.body = new FakeElement("body", this);
    this.listeners = new Map();
    this._observers = [];
  }

  createElement(tag, props) {
    return new FakeElement(tag, this, props);
  }

  createTemplate(rootElement) {
    const tpl = this.createElement("template");
    const fragment = this.createElement("fragment");
    fragment.append(rootElement);
    tpl.content = fragment;
    return tpl;
  }

  getElementById(id) {
    const walk = (node) => {
      for (const child of node.children) {
        if (child.id === id) return child;
        const found = walk(child);
        if (found) return found;
      }
      return null;
    };
    return walk(this.body);
  }

  querySelectorAll(selector) {
    return this.body.querySelectorAll(selector);
  }

  querySelector(selector) {
    return this.body.querySelector(selector);
  }

  addEventListener(type, fn) {
    if (!this.listeners.has(type)) this.listeners.set(type, []);
    this.listeners.get(type).push(fn);
  }

  dispatchEvent(event) {
    const ev = {
      defaultPrevented: false,
      preventDefault() {
        this.defaultPrevented = true;
      },
      ...event,
    };
    for (const fn of this.listeners.get(ev.type) ?? []) fn(ev);
    return !ev.defaultPrevented;
  }

  _notifyMutation(target, attributeName) {
    for (const obs of this._observers) {
      if (obs.target !== target) continue;
      if (obs.filter && !obs.filter.includes(attributeName)) continue;
      obs.callback([{ target, attributeName }]);
    }
  }
}

export function createEnvironment({
  hash = "",
  invoke = async () => ({}),
  storage = {},
  now = 1_700_000_000_000,
} = {}) {
  const document = new FakeDocument();
  const store = new Map(Object.entries(storage));
  const localStorage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, String(v)),
    removeItem: (k) => store.delete(k),
    clear: () => store.clear(),
  };

  let currentTime = now;
  let nextTimerId = 1;
  const timers = new Map();
  const intervals = new Map();

  const setTimeoutStub = (fn, ms = 0) => {
    const id = nextTimerId++;
    timers.set(id, { fn, due: currentTime + ms });
    return id;
  };
  const clearTimeoutStub = (id) => timers.delete(id);
  const setIntervalStub = (fn, ms = 0) => {
    const id = nextTimerId++;
    intervals.set(id, { fn, ms });
    return id;
  };
  const clearIntervalStub = (id) => intervals.delete(id);
  const rafQueue = [];

  const tauriEvents = new Map();
  const windowListeners = new Map();
  const invokeCalls = [];
  const clipboard = {
    written: [],
    fail: false,
    async writeText(text) {
      if (this.fail) throw new Error("clipboard denied");
      this.written.push(text);
    },
  };

  const location = { hash };
  const windowObj = {
    location,
    document,
    localStorage,
    navigator: { clipboard },
    __TAURI__: {
      core: {
        invoke: async (cmd, args) => {
          invokeCalls.push({ cmd, args });
          return invoke(cmd, args);
        },
      },
      event: {
        listen: (name, handler) => {
          if (!tauriEvents.has(name)) tauriEvents.set(name, []);
          tauriEvents.get(name).push(handler);
          return Promise.resolve(() => {});
        },
      },
    },
    addEventListener(type, fn) {
      if (!windowListeners.has(type)) windowListeners.set(type, []);
      windowListeners.get(type).push(fn);
    },
    dispatchEvent(type) {
      for (const fn of windowListeners.get(type) ?? []) fn({ type });
    },
  };

  class FakeMutationObserver {
    constructor(callback) {
      this.callback = callback;
    }
    observe(target, opts = {}) {
      document._observers.push({
        target,
        callback: this.callback,
        filter: opts.attributeFilter ?? null,
      });
    }
  }

  class FakeDate extends Date {
    constructor(...args) {
      super(...(args.length === 0 ? [currentTime] : args));
    }
    static now() {
      return currentTime;
    }
  }

  const sandbox = {
    window: windowObj,
    document,
    location,
    localStorage,
    navigator: windowObj.navigator,
    MutationObserver: FakeMutationObserver,
    Intl,
    JSON,
    Map,
    Set,
    Math,
    Number,
    String,
    Boolean,
    Array,
    Object,
    Promise,
    Error,
    Date: FakeDate,
    structuredClone,
    console,
    setTimeout: setTimeoutStub,
    clearTimeout: clearTimeoutStub,
    setInterval: setIntervalStub,
    clearInterval: clearIntervalStub,
    requestAnimationFrame: (fn) => rafQueue.push(fn),
  };
  const context = vm.createContext(sandbox);

  return {
    context,
    document,
    window: windowObj,
    location,
    localStorage,
    clipboard,
    invokeCalls,
    el(tag, props, ...children) {
      const node = document.createElement(tag, props);
      node.append(...children);
      return node;
    },
    load(...files) {
      for (const file of files) {
        const full = path.join(UI_DIR, file);
        const code = fs.readFileSync(full, "utf8");
        vm.runInContext(code, context, { filename: full });
      }
    },
    eval(expr) {
      return vm.runInContext(expr, context);
    },
    emitTauri(name, payload) {
      for (const handler of tauriEvents.get(name) ?? []) handler({ payload });
    },
    advanceTime(ms) {
      currentTime += ms;
      for (const [id, timer] of [...timers.entries()]) {
        if (timer.due <= currentTime) {
          timers.delete(id);
          timer.fn();
        }
      }
    },
    flushRaf() {
      while (rafQueue.length > 0) rafQueue.shift()();
    },
    tickIntervals() {
      for (const interval of intervals.values()) interval.fn();
    },
  };
}
