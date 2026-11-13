"use strict";

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

let text = {};
let saveTimer = null;
// What the Rust side says each style sets; the page never repeats the numbers.
let presets = [];
// The offsets as last saved or loaded, sent again when the ones typed in are not valid.
let savedOffsets = {};
// Set when the user picks a place (or a style sets one); only then is a place sent, so a Ctrl drag made after the
// page loaded is not undone by the next slider move.
let placeChosen = false;

function showError(on) {
  const box = $("error");
  box.hidden = !on;
  if (on) box.textContent = text["settings.error"] ?? "";
}

function renderOffsets(apps) {
  savedOffsets = Object.fromEntries(apps.filter((a) => a.offset_ms !== 0).map((a) => [a.id, a.offset_ms]));
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

/** A radio group drawn as buttons: the input, then its label, one pair per option. */
function renderChoices(box, name, options, onLabel) {
  box.replaceChildren(
    ...options.flatMap((o) => {
      const input = document.createElement("input");
      input.type = "radio";
      input.name = name;
      input.id = `${name}-${o.id}`;
      input.value = o.id;
      const label = document.createElement("label");
      label.htmlFor = input.id;
      onLabel(label, o);
      return [input, label];
    }),
  );
}

function checkRadio(name, value) {
  for (const input of document.getElementsByName(name)) input.checked = input.value === String(value);
}

function checked(name) {
  return document.querySelector(`input[name="${name}"]:checked`)?.value;
}

function showSliders() {
  const shown = {
    size: `${$("size").value} ${text["unit.px"] ?? ""}`,
    plate: `${$("plate").value}%`,
    width: `${$("width").value}%`,
  };
  for (const [id, value] of Object.entries(shown)) {
    $(`${id}-out`).textContent = value;
    $(id).setAttribute("aria-valuetext", value); // the value with its unit, for screen readers
  }
}

/** Puts the display values into the controls. */
function fillLook(look) {
  $("size").value = String(look.font_px);
  $("plate").value = String(look.plate_opacity);
  $("width").value = String(look.max_width_pct);
  checkRadio("lines", look.lines);
  checkRadio("accent", look.accent);
  $("place").value = look.placement;
  showSliders();
}

function markPreset(id) {
  checkRadio("preset", id);
  $("preset-custom").textContent = id === "custom" ? (text["preset.custom"] ?? "") : "";
}

let built = false;

function renderLook(view) {
  presets = view.presets;
  // Built once: the window reloads on every focus, and rebuilding would take the keyboard focus away.
  if (!built) {
    built = true;
    renderChoices($("presets"), "preset", presets, (label, p) => (label.textContent = text[`preset.${p.id}`] ?? ""));
    renderChoices($("accents"), "accent", view.accents, (label, a) => {
      const dot = document.createElement("span");
      dot.className = "swatch";
      dot.style.background = a.hex;
      dot.setAttribute("aria-hidden", "true");
      label.append(dot, document.createTextNode(text[`accent.${a.id}`] ?? ""));
    });
  }
  const ids = [...view.placements];
  // A strip that was dragged by hand has no named place: shown, but not something to choose.
  if (view.placement === "custom") ids.push("custom");
  if ($("place").dataset.ids !== ids.join()) {
    $("place").dataset.ids = ids.join();
    $("place").replaceChildren(
      ...ids.map((id) => {
        const option = document.createElement("option");
        option.value = id;
        option.disabled = id === "custom";
        option.textContent = text[`place.${id}`] ?? "";
        return option;
      }),
    );
  }
  fillLook(view);
  markPreset(view.preset);
}

async function load() {
  const view = await invoke("get_settings");
  renderLook(view);
  $("capture").checked = view.hide_from_capture;
  $("version").textContent = view.version;
  renderOffsets(view.apps);
  renderShortcuts(view.shortcuts);
  $("autostart").checked = await invoke("plugin:autostart|is_enabled").catch(() => false);
}

/** Sends the form as it is. The server checks every limit again; a refusal puts the saved values back.
 *  An offset that is not valid is not sent (the saved ones go instead) and does not hold back the display settings. */
async function save() {
  let offsets = {};
  let offsetsValid = true;
  for (const li of $("offsets").children) {
    const input = li.querySelector("input");
    if (!input.checkValidity()) {
      offsetsValid = false;
      break;
    }
    const ms = Math.round(Number(input.value) || 0);
    if (ms !== 0) offsets[li.dataset.app] = ms;
  }
  if (!offsetsValid) offsets = savedOffsets;
  try {
    const preset = await invoke("set_settings", {
      input: {
        font_px: Number($("size").value),
        hide_from_capture: $("capture").checked,
        app_offsets_ms: offsets,
        placement: placeChosen ? $("place").value : null,
        lines: Number(checked("lines")),
        plate_opacity: Number($("plate").value),
        max_width_pct: Number($("width").value),
        accent: checked("accent"),
      },
    });
    markPreset(preset);
    placeChosen = false;
    if (offsetsValid) savedOffsets = offsets;
    showError(!offsetsValid);
  } catch {
    showError(true);
    await load();
  }
}

for (const id of ["size", "plate", "width"]) {
  $(id).addEventListener("input", () => {
    showSliders();
    // The strip follows while the slider moves; saved at most every 80 ms.
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 80);
  });
  $(id).addEventListener("change", () => {
    clearTimeout(saveTimer);
    save();
  });
}
$("place").addEventListener("change", () => {
  placeChosen = true;
  save();
});
$("accents").addEventListener("change", save);
for (const input of document.getElementsByName("lines")) input.addEventListener("change", save);
// A style sets every display value at once, in one save.
$("presets").addEventListener("change", () => {
  const look = presets.find((p) => p.id === checked("preset"));
  if (look) fillLook(look);
  placeChosen = true;
  save();
});
$("reset-look").addEventListener("click", () => {
  const base = presets.find((p) => p.id === "default");
  if (base) fillLook(base);
  placeChosen = true;
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
