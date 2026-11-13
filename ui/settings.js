"use strict";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

let text = {};
let saveTimer = null;

function showError(on) {
  const box = $("error");
  box.hidden = !on;
  if (on) box.textContent = text["settings.error"] ?? "";
}

function renderOffsets(apps) {
  $("offsets-empty").hidden = apps.length > 0;
  $("offsets").replaceChildren(
    ...apps.map((app, i) => {
      const li = document.createElement("li");
      li.dataset.app = app.id;
      const label = document.createElement("label");
      label.htmlFor = `offset-${i}`;
      label.textContent = app.id;
      const input = document.createElement("input");
      input.id = `offset-${i}`;
      input.type = "number";
      input.min = "-2000";
      input.max = "2000";
      input.step = "50";
      input.value = String(app.offset_ms);
      const unit = document.createElement("span");
      unit.className = "unit";
      unit.textContent = text["unit.ms"] ?? "";
      li.append(label, input, unit);
      return li;
    }),
  );
}

function renderShortcuts(list) {
  $("shortcuts").replaceChildren(
    ...list.map((s) => {
      const li = document.createElement("li");
      const name = document.createElement("span");
      name.textContent = text[s.label] ?? "";
      const keys = document.createElement("kbd");
      keys.textContent = s.keys;
      li.append(name, keys);
      if (!s.available) {
        keys.classList.add("gone");
        const off = document.createElement("span");
        off.className = "off";
        off.textContent = text["settings.shortcut.unavailable"] ?? "";
        li.append(off);
      }
      return li;
    }),
  );
}

async function load() {
  const view = await invoke("get_settings");
  $("size").value = String(view.font_px);
  $("size-out").textContent = `${view.font_px} ${text["unit.px"] ?? ""}`;
  $("capture").checked = view.hide_from_capture;
  $("version").textContent = view.version;
  renderOffsets(view.apps);
  renderShortcuts(view.shortcuts);
  $("autostart").checked = await invoke("plugin:autostart|is_enabled").catch(() => false);
}

/** Sends the form as it is. The server checks every limit again; a refusal puts the saved values back. */
async function save() {
  const offsets = {};
  for (const li of $("offsets").children) {
    const input = li.querySelector("input");
    if (!input.checkValidity()) {
      showError(true);
      return;
    }
    const ms = Math.round(Number(input.value) || 0);
    if (ms !== 0) offsets[li.dataset.app] = ms;
  }
  try {
    await invoke("set_settings", {
      input: { font_px: Number($("size").value), hide_from_capture: $("capture").checked, app_offsets_ms: offsets },
    });
    showError(false);
  } catch {
    showError(true);
    await load();
  }
}

$("size").addEventListener("input", () => {
  $("size-out").textContent = `${$("size").value} ${text["unit.px"] ?? ""}`;
  // The strip follows while the slider moves; saved at most every 80 ms.
  clearTimeout(saveTimer);
  saveTimer = setTimeout(save, 80);
});
$("size").addEventListener("change", () => {
  clearTimeout(saveTimer);
  save();
});
$("capture").addEventListener("change", save);
$("offsets").addEventListener("change", save);
$("autostart").addEventListener("change", async () => {
  const command = $("autostart").checked ? "enable" : "disable";
  await invoke(`plugin:autostart|${command}`).catch(() => {});
  // The box shows what Windows now has, whether or not the change worked.
  $("autostart").checked = await invoke("plugin:autostart|is_enabled").catch(() => false);
});

// The tray can change the same things while this window is open.
window.addEventListener("focus", () => load().catch(() => {}));

async function start() {
  text = await invoke("strings");
  document.documentElement.lang = text.lang ?? "en";
  for (const el of document.querySelectorAll("[data-t]")) el.textContent = text[el.dataset.t] ?? "";
  document.title = text["settings.title"] ?? "Encore";
  await load();
}

start();
