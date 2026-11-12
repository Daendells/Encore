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

function setRow(row, text) {
  row.hidden = text == null;
  if (text != null) row.textContent = text;
}

/** Where a visible row is now: vertical centre, opacity and font size. */
function look(row) {
  if (row.hidden) return null;
  const box = row.getBoundingClientRect();
  const css = getComputedStyle(row);
  return { y: box.top + box.height / 2, h: box.height, opacity: css.opacity, size: parseFloat(css.fontSize) };
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
  plate.classList.remove("slide");
  // After a step forward the old current row becomes prev, the old next row becomes current.
  const old = slide ? { cur: look(rows.cur), next: look(rows.next) } : null;

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
      // The row starts where its text was, at the size and opacity it had, then moves to its own place.
      const dy = src ? src.y - now.y : now.h;
      const scale = src ? src.size / now.size : 1;
      row.style.transform = `translateY(${dy}px) scale(${scale})`;
      row.style.opacity = src ? src.opacity : 0;
    }
  }
  if (old || pending.length) void plate.offsetWidth; // commit the start state before the end state is set
  if (old) {
    plate.classList.add("slide");
    for (const row of Object.values(rows)) {
      row.style.transform = "";
      row.style.opacity = "";
    }
  }
  for (const span of pending) span.classList.add("on");

  showingLines = true;
  if (window.__LARIK_TIMING__ === true) invoke("shown", { seq: ui.seq }).catch(() => {});
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
});

listen("style", (event) => {
  const px = Number(event.payload.font_px);
  if (Number.isFinite(px) && px > 0) document.documentElement.style.setProperty("--font", `${px}px`);
});
