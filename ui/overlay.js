"use strict";

const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

const plate = document.getElementById("plate");
const toast = document.getElementById("toast");
const rows = {
  prev: document.getElementById("prev"),
  cur: document.getElementById("cur"),
  next: document.getElementById("next"),
};
const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
// Whether the plate currently shows lyric lines (a slide only makes sense from one line set to the next).
let showingLines = false;

// The first size comes with the page (an event sent before this script listens would be lost).
const firstPx = Number(window.__ENCORE_FONT_PX__);
if (Number.isFinite(firstPx) && firstPx > 0) document.documentElement.style.setProperty("--font", `${firstPx}px`);

function setRow(row, text) {
  row.hidden = text == null;
  if (text != null) row.textContent = text;
}

/** Where a visible row is on screen now, in-flight transforms included: centre, height, colour, opacity, effective text size. */
function look(row) {
  if (row.hidden) return null;
  const box = row.getBoundingClientRect();
  const css = getComputedStyle(row);
  // The box is scaled by a running slide, the layout height (offsetHeight) is not.
  const size = (parseFloat(css.fontSize) * box.height) / row.offsetHeight;
  return { y: box.top + box.height / 2, h: box.height, color: css.color, opacity: css.opacity, size };
}

/** The current line as one span per word, dim until its turn. Returns the spans that still have to light up. */
function setWords(words) {
  const pending = [];
  const spans = words.map((w) => {
    const span = document.createElement("span");
    span.className = "w";
    span.textContent = w.text;
    if (w.delay_ms < 0) {
      span.classList.add("on");
    } else {
      span.style.transitionDelay = `${Math.max(0, w.delay_ms)}ms`;
      span.style.transitionDuration = `${Math.min(w.dur_ms, 300)}ms`;
      pending.push(span);
    }
    return span;
  });
  rows.cur.replaceChildren(...spans);
  return pending;
}

function showLines(ui) {
  const slide = ui.animate && showingLines && !reduceMotion.matches;
  // Measured before anything changes, so a slide still running is measured where it is on screen.
  // After a step forward the old current row becomes prev, the old next row becomes current.
  const old = slide ? { cur: look(rows.cur), next: look(rows.next) } : null;
  plate.classList.remove("slide"); // also ends any running transition

  plate.hidden = false;
  setRow(rows.prev, ui.prev);
  setRow(rows.next, ui.next);
  rows.cur.classList.toggle("dim", !ui.lit);
  let pending = [];
  if (ui.words && ui.words.length) pending = setWords(ui.words);
  else rows.cur.textContent = ui.cur;

  if (old) {
    const from = { prev: old.cur, cur: old.next, next: null };
    for (const [name, row] of Object.entries(rows)) {
      const now = look(row);
      if (!now) continue;
      const src = from[name];
      // The row starts where its text was, at the size, colour and opacity it had, then moves to its own place.
      const dy = src ? src.y - now.y : now.h;
      const scale = src ? src.size / now.size : 1;
      row.style.transform = `translateY(${dy}px) scale(${scale})`;
      row.style.opacity = src ? src.opacity : 0;
      if (src) row.style.color = src.color;
    }
  }
  if (old || pending.length) void plate.offsetWidth; // commit the start state before the end state is set
  if (old) {
    plate.classList.add("slide");
    for (const row of Object.values(rows)) {
      row.style.transform = "";
      row.style.opacity = "";
      row.style.color = "";
    }
  }
  for (const span of pending) span.classList.add("on");

  showingLines = true;
  if (window.__ENCORE_TIMING__ === true) invoke("shown", { seq: ui.seq }).catch(() => {});
}

function showInfo(ui) {
  plate.classList.remove("slide");
  plate.hidden = false;
  setRow(rows.prev, null);
  rows.cur.classList.remove("dim");
  setRow(rows.cur, ui.title);
  setRow(rows.next, ui.artist);
  showingLines = false;
}

/** Nothing of the last song stays on the plate for the next time the window is shown. */
function clearPlate() {
  plate.classList.remove("slide");
  plate.hidden = true;
  for (const row of Object.values(rows)) setRow(row, null);
  showingLines = false;
}

function showToast(ui) {
  toast.classList.remove("go");
  toast.textContent = ui.text;
  toast.hidden = false;
  void toast.offsetWidth; // restart the animation if one is still running
  toast.classList.add("go");
}

toast.addEventListener("animationend", () => {
  toast.classList.remove("go");
  toast.hidden = true;
});

listen("ui", (event) => {
  const ui = event.payload;
  if (ui.kind === "lines") showLines(ui);
  else if (ui.kind === "info") showInfo(ui);
  else if (ui.kind === "toast") showToast(ui);
  else if (ui.kind === "clear") clearPlate();
});

listen("style", (event) => {
  const px = Number(event.payload.font_px);
  if (Number.isFinite(px) && px > 0) document.documentElement.style.setProperty("--font", `${px}px`);
});

// While Ctrl is held the window takes mouse input and the plate can be dragged.
listen("drag", (event) => document.body.classList.toggle("drag", event.payload.on === true));

// A key-up that never reached Encore must not leave the strip clickable: a mouse event without Ctrl ends the drag state.
function ctrlGone(event) {
  if (event.ctrlKey || !document.body.classList.contains("drag")) return false;
  invoke("ctrl_released").catch(() => {});
  return true;
}

document.addEventListener("mousemove", ctrlGone);
document.addEventListener("mousedown", (event) => {
  if (!ctrlGone(event) && event.button === 0 && document.body.classList.contains("drag")) {
    window.__TAURI__.window.getCurrentWindow().startDragging().catch(() => {});
  }
});
