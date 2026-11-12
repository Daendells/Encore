// Usage: node scripts/timing-report.mjs <timing-log.csv>
// Reads the CSV written when LARIK_TIMING_LOG is set (kind,wall_ms,app,title,line_index,lrc_ms,pos_ms,extra).
// Prints, per app and overall: line timing error (pos_ms - lrc_ms) and clock drift (extra of numeric `sync` rows).
// Only `shown` rows from the line timer count (extra "<seq> timer"). Rows from pause/resume, hide/show, nudge,
// seek, settings reload or the first line of a song (extra "<seq> other") are not line starts and are excluded.
import { readFileSync } from "node:fs";

const file = process.argv[2];
if (!file) {
  console.error("usage: node scripts/timing-report.mjs <timing-log.csv>");
  process.exit(1);
}

// Splits CSV text into rows of cells; handles quoted cells with commas and doubled quotes.
function parseCsv(text) {
  const rows = [];
  let row = [], cell = "", quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') { cell += '"'; i++; }
      else if (c === '"') quoted = false;
      else cell += c;
    } else if (c === '"') quoted = true;
    else if (c === ",") { row.push(cell); cell = ""; }
    else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(cell); cell = "";
      if (row.length > 1 || row[0] !== "") rows.push(row);
      row = [];
    } else cell += c;
  }
  if (cell !== "" || row.length) { row.push(cell); rows.push(row); }
  return rows;
}

const [header, ...rows] = parseCsv(readFileSync(file, "utf8").replace(/^﻿/, ""));
const col = Object.fromEntries(header.map((name, i) => [name, i]));
const apps = new Map(); // app -> { err: [], drift: [], other: 0, unmarked: 0 }
const bucket = (app) => {
  if (!apps.has(app)) apps.set(app, { err: [], drift: [], other: 0, unmarked: 0 });
  return apps.get(app);
};

for (const r of rows) {
  const kind = r[col.kind], app = r[col.app], extra = r[col.extra];
  if (kind === "sync") {
    if (extra !== "seek" && extra !== "first" && extra !== "" && Number.isFinite(Number(extra))) bucket(app).drift.push(Math.abs(Number(extra)));
  } else if (kind === "shown" && r[col.lrc_ms] !== "") {
    const b = bucket(app);
    const marker = (extra ?? "").split(" ")[1];
    if (marker === "timer") b.err.push(Number(r[col.pos_ms]) - Number(r[col.lrc_ms]));
    else if (marker === "other") b.other++;
    else b.unmarked++;
  }
}

const mean = (a) => a.reduce((s, x) => s + x, 0) / a.length;
const p95 = (a) => {
  const s = [...a].sort((x, y) => x - y);
  return s[Math.max(0, Math.ceil(0.95 * s.length) - 1)]; // nearest rank
};
const f = (x) => x.toFixed(1);

function report(name, { err, drift, other, unmarked }) {
  console.log(`\n== ${name} ==`);
  if (err.length) {
    const abs = err.map(Math.abs);
    console.log(`lines shown by the timer: n=${err.length}`);
    console.log(`  error (ms after LRC time): mean=${f(mean(err))}  mean|e|=${f(mean(abs))}  p95|e|=${f(p95(abs))}  max|e|=${f(Math.max(...abs))}`);
  } else console.log("lines shown by the timer: n=0");
  console.log(`excluded: ${other} not from the line timer (pause/resume, show/hide, nudge, seek, settings, first line), ${unmarked} without a marker (old log format)`);
  if (drift.length) console.log(`clock drift at sync (ms): n=${drift.length}  mean|d|=${f(mean(drift))}  p95|d|=${f(p95(drift))}`);
  else console.log("clock drift at sync: n=0");
}

const all = { err: [], drift: [], other: 0, unmarked: 0 };
for (const [app, b] of apps) {
  report(app, b);
  all.err.push(...b.err); all.drift.push(...b.drift); all.other += b.other; all.unmarked += b.unmarked;
}
report("overall", all);
console.log("\nTargets: mean|e| < 150 ms and p95|e| < 300 ms.");
