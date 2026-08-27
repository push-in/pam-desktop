(() => {
  "use strict";
  if (window.PamWorkstation) return;
  const requireValue = (condition, message) => { if (!condition) throw new TypeError(message); };

  class CommandRegistry extends EventTarget {
    #commands = new Map(); #shortcuts = new Map();
    register(value) {
      requireValue(value && typeof value.id === "string" && typeof value.run === "function", "A command requires id and run().");
      if (this.#commands.has(value.id)) throw new Error(`Command '${value.id}' is already registered.`);
      const command = Object.freeze({ title: value.id, category: "General", enabled: () => true, visible: () => true, ...value });
      this.#commands.set(command.id, command);
      for (const shortcut of command.shortcuts ?? []) {
        const key = CommandRegistry.normalize(shortcut);
        if (this.#shortcuts.has(key)) throw new Error(`Shortcut conflict between '${command.id}' and '${this.#shortcuts.get(key)}'.`);
        this.#shortcuts.set(key, command.id);
      }
      this.dispatchEvent(new CustomEvent("change", { detail: { type: 1, id: command.id } }));
      return () => this.unregister(command.id);
    }
    unregister(id) { if (!this.#commands.delete(id)) return false; for (const [key, value] of this.#shortcuts) if (value === id) this.#shortcuts.delete(key); this.dispatchEvent(new CustomEvent("change", { detail: { type: 2, id } })); return true; }
    list(context = {}) { return [...this.#commands.values()].filter((item) => item.visible(context)); }
    search(query, context = {}) { const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean); return this.list(context).map((command) => { const text = `${command.title} ${command.category} ${command.id} ${(command.keywords ?? []).join(" ")}`.toLowerCase(); return { command, score: terms.reduce((score, term) => score + (text.startsWith(term) ? 4 : text.includes(term) ? 1 : -20), 0) }; }).filter((item) => item.score >= 0).sort((a, b) => b.score - a.score || a.command.title.localeCompare(b.command.title)).map((item) => item.command); }
    async execute(id, context = {}) { const command = this.#commands.get(id); if (!command) throw new Error(`Unknown command '${id}'.`); if (!command.enabled(context)) return false; await command.run(context); this.dispatchEvent(new CustomEvent("execute", { detail: { id } })); return true; }
    handleKeydown(event, context = {}) { const id = this.#shortcuts.get(CommandRegistry.fromEvent(event)); if (!id) return false; event.preventDefault(); void this.execute(id, context); return true; }
    static normalize(value) { return value.toLowerCase().replaceAll("commandorcontrol", navigator.platform.includes("Mac") ? "meta" : "control").split("+").map((part) => part.trim()).sort().join("+"); }
    static fromEvent(event) { return [event.altKey && "alt", event.ctrlKey && "control", event.metaKey && "meta", event.shiftKey && "shift", event.code.toLowerCase()].filter(Boolean).sort().join("+"); }
  }

  class UndoManager extends EventTarget {
    #undo = []; #redo = []; #transaction = null;
    constructor(limit = 500) { super(); this.limit = limit; }
    begin(label = "Change") { if (this.#transaction) throw new Error("An undo transaction is already open."); this.#transaction = { label, operations: [] }; }
    record(redo, undo) { requireValue(typeof redo === "function" && typeof undo === "function", "Undo operations must be functions."); const operation = { redo, undo }; if (this.#transaction) this.#transaction.operations.push(operation); else this.#commit({ label: "Change", operations: [operation] }); }
    commit() { const value = this.#transaction; this.#transaction = null; if (value?.operations.length) this.#commit(value); }
    rollback() { const value = this.#transaction; this.#transaction = null; for (const item of [...(value?.operations ?? [])].reverse()) item.undo(); this.#notify(); }
    async undo() { const value = this.#undo.pop(); if (!value) return false; for (const item of [...value.operations].reverse()) await item.undo(); this.#redo.push(value); this.#notify(); return true; }
    async redo() { const value = this.#redo.pop(); if (!value) return false; for (const item of value.operations) await item.redo(); this.#undo.push(value); this.#notify(); return true; }
    get state() { return Object.freeze({ canUndo: !!this.#undo.length, canRedo: !!this.#redo.length, undoLabel: this.#undo.at(-1)?.label ?? null, redoLabel: this.#redo.at(-1)?.label ?? null }); }
    #commit(value) { this.#undo.push(value); if (this.#undo.length > this.limit) this.#undo.shift(); this.#redo.length = 0; this.#notify(); }
    #notify() { this.dispatchEvent(new CustomEvent("change", { detail: this.state })); }
  }

  class RenderScheduler {
    #regions = new Map(); #frame = 0;
    invalidate(key, render) { this.#regions.set(key, render); if (!this.#frame) this.#frame = requestAnimationFrame(() => this.#flush()); }
    cancel(key) { this.#regions.delete(key); }
    #flush() { this.#frame = 0; const batch = [...this.#regions.values()]; this.#regions.clear(); for (const render of batch) render(); }
  }

  class VirtualViewport {
    static MAX_SCROLL_SIZE = 8000000;
    constructor({ count = 0, estimateSize = 32, overscan = 8 } = {}) { requireValue(Number.isSafeInteger(count) && count >= 0 && Number.isFinite(estimateSize) && estimateSize > 0 && Number.isSafeInteger(overscan) && overscan >= 0, "Virtual viewport options are invalid."); this.count = count; this.estimateSize = estimateSize; this.overscan = overscan; }
    range(offset, size) { const start = Math.max(0, Math.floor(offset / this.estimateSize) - this.overscan); const end = Math.min(this.count, start + Math.ceil(size / this.estimateSize) + this.overscan * 2); return Object.freeze({ start, end, offset: start * this.estimateSize, totalSize: this.count * this.estimateSize }); }
    mount(container, renderItem) { requireValue(container instanceof Element && typeof renderItem === "function", "Virtual viewport requires an element and renderer."); container.classList.add("pam-virtual-viewport"); const canvas = document.createElement("div"); canvas.className = "pam-virtual-canvas"; container.replaceChildren(canvas); const mounted = new Map(); let frame = 0; const render = () => { frame = 0; const total = this.count * this.estimateSize; const physical = Math.min(total, VirtualViewport.MAX_SCROLL_SIZE); canvas.style.height = `${physical}px`; const physicalRange = Math.max(0, physical - container.clientHeight); const logicalRange = Math.max(0, total - container.clientHeight); const logicalOffset = physicalRange === 0 ? 0 : (container.scrollTop / physicalRange) * logicalRange; const range = this.range(logicalOffset, container.clientHeight); const next = new Map(); const fragment = document.createDocumentFragment(); for (let index = range.start; index < range.end; index++) { const row = mounted.get(index) ?? renderItem(index); requireValue(row instanceof Element, "Virtual item renderers must return an Element."); row.style.position = "absolute"; row.style.insetInline = "0"; row.style.transform = `translateY(${container.scrollTop + index * this.estimateSize - logicalOffset}px)`; row.style.height = `${this.estimateSize}px`; row.setAttribute("aria-posinset", index + 1); row.setAttribute("aria-setsize", this.count); next.set(index, row); fragment.append(row); } mounted.clear(); for (const [index, row] of next) mounted.set(index, row); canvas.replaceChildren(fragment); }; const schedule = () => { if (!frame) frame = requestAnimationFrame(render); }; container.addEventListener("scroll", schedule, { passive: true }); const observer = new ResizeObserver(schedule); observer.observe(container); render(); return { refresh: schedule, scrollToIndex: (index, behavior = "auto") => { requireValue(Number.isSafeInteger(index) && index >= 0 && index < this.count, "Virtual index is outside the collection."); const total = this.count * this.estimateSize; const physical = Math.min(total, VirtualViewport.MAX_SCROLL_SIZE); const logicalRange = Math.max(1, total - container.clientHeight); const physicalRange = Math.max(0, physical - container.clientHeight); container.scrollTo({ top: Math.min(physicalRange, index * this.estimateSize / logicalRange * physicalRange), behavior }); }, destroy: () => { if (frame) cancelAnimationFrame(frame); observer.disconnect(); container.removeEventListener("scroll", schedule); mounted.clear(); container.replaceChildren(); } }; }
  }

  class RecoveryJournal {
    constructor(namespace, { delay = 750, maxEntries = 25 } = {}) { requireValue(namespace, "Recovery journal requires a namespace."); this.key = `pam:recovery:${namespace}`; this.delay = delay; this.maxEntries = maxEntries; }
    save(documentId, value) { clearTimeout(this.timer); this.timer = setTimeout(() => { const entries = this.readAll(); entries.unshift({ documentId, value, savedAt: Date.now() }); localStorage.setItem(this.key, JSON.stringify(entries.slice(0, this.maxEntries))); }, this.delay); }
    readAll() { try { const value = JSON.parse(localStorage.getItem(this.key) ?? "[]"); return Array.isArray(value) ? value : []; } catch { return []; } }
    recover(documentId) { return this.readAll().find((entry) => entry.documentId === documentId) ?? null; }
    clear(documentId = null) { if (documentId === null) localStorage.removeItem(this.key); else localStorage.setItem(this.key, JSON.stringify(this.readAll().filter((entry) => entry.documentId !== documentId))); }
  }

  class WorkspaceStore {
    constructor(namespace) { this.key = `pam:workspace:${namespace}`; }
    save(state) { localStorage.setItem(this.key, JSON.stringify({ version: 1, state, savedAt: Date.now() })); }
    restore(fallback = null) { try { return JSON.parse(localStorage.getItem(this.key) ?? "null")?.state ?? fallback; } catch { return fallback; } }
  }

  class FocusManager {
    static trap(container) { const listener = (event) => { if (event.key !== "Tab") return; const items = [...container.querySelectorAll('button,[href],input,select,textarea,[tabindex]:not([tabindex="-1"])')].filter((node) => !node.disabled && !node.hidden); if (!items.length) return; const first = items[0], last = items.at(-1); if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); } else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); } }; container.addEventListener("keydown", listener); return () => container.removeEventListener("keydown", listener); }
  }

  class PerformanceGate {
    constructor(budgets = {}) { this.budgets = { frameP95Ms: 16.67, ipcP95Ms: 5, ...budgets }; this.samples = new Map(); }
    sample(metric, milliseconds) { const values = this.samples.get(metric) ?? []; values.push(milliseconds); if (values.length > 1000) values.shift(); this.samples.set(metric, values); }
    report() { const metrics = {}; for (const [name, values] of this.samples) { const sorted = [...values].sort((a, b) => a - b); metrics[name] = { count: values.length, p50: sorted[Math.floor(sorted.length * .5)] ?? 0, p95: sorted[Math.floor(sorted.length * .95)] ?? 0 }; } const violations = []; if ((metrics.frame?.p95 ?? 0) > this.budgets.frameP95Ms) violations.push("frameP95Ms"); if ((metrics.ipc?.p95 ?? 0) > this.budgets.ipcP95Ms) violations.push("ipcP95Ms"); return Object.freeze({ passed: !violations.length, violations, metrics }); }
  }

  class DockLayout extends EventTarget {
    constructor(root, state = { direction: 1, panels: [] }) { super(); requireValue(root instanceof Element, "Dock layout requires an element."); this.root = root; this.state = structuredClone(state); this.render(); }
    setState(state) { this.state = structuredClone(state); this.render(); this.dispatchEvent(new CustomEvent("change", { detail: this.state })); }
    move(panelId, targetIndex) { const panels = [...this.state.panels]; const index = panels.findIndex((panel) => panel.id === panelId); if (index < 0) return false; const [panel] = panels.splice(index, 1); panels.splice(Math.max(0, Math.min(targetIndex, panels.length)), 0, panel); this.setState({ ...this.state, panels }); return true; }
    resize(panelId, basis) { this.setState({ ...this.state, panels: this.state.panels.map((panel) => panel.id === panelId ? { ...panel, basis: Math.max(80, basis) } : panel) }); }
    render() { this.root.classList.add("pam-dock"); this.root.style.flexDirection = this.state.direction === 2 ? "column" : "row"; const fragment = document.createDocumentFragment(); for (const panel of this.state.panels) { const node = document.createElement("section"); node.className = "pam-panel"; node.dataset.panelId = panel.id; node.style.flexBasis = `${panel.basis ?? 240}px`; node.setAttribute("aria-label", panel.title ?? panel.id); node.append(document.createElement("slot")); node.querySelector("slot").name = panel.id; fragment.append(node); } this.root.replaceChildren(fragment); }
  }

  class DetachableTabs extends EventTarget {
    constructor(tabs = []) { super(); this.tabs = [...tabs]; this.activeId = tabs[0]?.id ?? null; }
    activate(id) { if (!this.tabs.some((tab) => tab.id === id)) return false; this.activeId = id; this.dispatchEvent(new CustomEvent("change", { detail: this.snapshot() })); return true; }
    reorder(id, index) { const current = this.tabs.findIndex((tab) => tab.id === id); if (current < 0) return false; const [tab] = this.tabs.splice(current, 1); this.tabs.splice(Math.max(0, Math.min(index, this.tabs.length)), 0, tab); this.dispatchEvent(new CustomEvent("change", { detail: this.snapshot() })); return true; }
    detach(id, windowId) { const tab = this.tabs.find((item) => item.id === id); if (!tab) return false; this.dispatchEvent(new CustomEvent("detach", { detail: { tab, windowId } })); return true; }
    snapshot() { return Object.freeze({ activeId: this.activeId, tabs: this.tabs.map((tab) => ({ ...tab })) }); }
  }

  class CommandPalette {
    constructor(registry) { this.registry = registry; }
    open(context = {}) { if (this.dialog?.isConnected) return; const dialog = document.createElement("dialog"); dialog.className = "pam-command-palette"; dialog.setAttribute("aria-label", "Command palette"); const input = document.createElement("input"); input.type = "search"; input.placeholder = "Type a command"; input.setAttribute("aria-label", "Search commands"); const results = document.createElement("div"); results.role = "listbox"; const draw = () => { const commands = this.registry.search(input.value, context).slice(0, 100); results.replaceChildren(...commands.map((command, index) => { const button = document.createElement("button"); button.type = "button"; button.role = "option"; button.tabIndex = index === 0 ? 0 : -1; button.textContent = `${command.category} · ${command.title}`; button.addEventListener("click", async () => { await this.registry.execute(command.id, context); dialog.close(); }); return button; })); }; input.addEventListener("input", draw); dialog.addEventListener("close", () => dialog.remove(), { once: true }); dialog.append(input, results); document.body.append(dialog); this.dialog = dialog; FocusManager.trap(dialog); dialog.showModal(); draw(); input.focus(); }
    close() { this.dialog?.close(); }
  }

  const registerComposerCommands = (pam, registry = commands) => {
    requireValue(pam?.commands && typeof pam.commands.list === "function", "PAM command metadata is unavailable.");
    return pam.commands.list().map((command) => registry.register({
      id: command.name,
      title: command.name,
      category: "Application",
      run: (context = {}) => pam.commands.invoke(command.name, context.payload ?? null, context.options ?? {}),
    }));
  };

  const commands = new CommandRegistry();
  const api = Object.freeze({ version: "1.0.0", commands, palette: new CommandPalette(commands), undo: new UndoManager(), render: new RenderScheduler(), CommandRegistry, UndoManager, RenderScheduler, VirtualViewport, RecoveryJournal, WorkspaceStore, FocusManager, PerformanceGate, DockLayout, DetachableTabs, CommandPalette, registerComposerCommands, createVirtualList: (container, options, renderer) => new VirtualViewport(options).mount(container, renderer), announce(message) { let region = document.querySelector("#pam-live-region"); if (!region) { region = document.createElement("div"); region.id = "pam-live-region"; region.className = "pam-visually-hidden"; region.setAttribute("role", "status"); region.setAttribute("aria-live", "polite"); document.body.append(region); } region.textContent = ""; requestAnimationFrame(() => region.textContent = message); } });
  Object.defineProperty(window, "PamWorkstation", { value: api });
  document.addEventListener("keydown", (event) => api.commands.handleKeydown(event));
})();
