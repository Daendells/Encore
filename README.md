# Encore

Encore menampilkan lirik lagu yang sedang Anda putar di Windows. Liriknya muncul sebagai strip tiga baris tepat di atas taskbar, selalu di depan jendela lain, dan klik menembusnya. Baris yang sedang dinyanyikan menyala sesuai waktu lagu.

![Strip lirik Encore di atas taskbar](docs/screenshot.png)

Bahasa Indonesia ada di bawah ini, [English version below](#english).

## Cara kerja

1. Encore membaca judul, artis, album, durasi, posisi, dan status putar dari kontrol media Windows (SMTC). Tidak ada login dan tidak memakai API Spotify. Pada uji, Spotify desktop dan Chrome (YouTube, YouTube Music) memberi data ini.
2. Judul dan artis dibersihkan dari hiasan seperti "(Remastered 2011)", "feat.", dan "(Official Video)". Encore lalu meminta lirik bertimbang waktu ke [LRCLIB](https://lrclib.net) dengan judul, artis, album, dan durasi.
3. Kalau permintaan pertama gagal, Encore mencari lalu memberi skor pada setiap kandidat dari kemiripan judul dan artis. Durasinya harus berselisih paling banyak 2 detik. Kalau tidak ada yang lolos, lirik dianggap tidak ditemukan. Encore dirancang untuk menolak lirik lagu lain: kalau ragu, lagunya dianggap tidak ditemukan.
4. Posisi lagu dihitung dari laporan terakhir pemutar ditambah waktu yang berlalu sejak laporan itu. Jam ini disinkronkan ulang setiap kali pemutar melapor. Lompatan lebih dari 1,5 detik dianggap seek.
5. Satu timer dibuat untuk baris berikutnya dan dihitung ulang setiap sinkronisasi. Tidak ada polling cepat. Saat strip disembunyikan atau tidak ada lagu, tidak ada timer yang jalan dan jendela strip tidak menggambar. Pendengar SMTC tetap aktif, dan lagu baru tetap dicari liriknya supaya siap begitu strip ditampilkan lagi.
6. Jendela strip (WebView2) hanya menggambar. Pergantian baris memakai transisi CSS 180 ms, dan transisi dimatikan kalau Windows diatur untuk mengurangi gerakan. Lirik yang punya waktu per kata (enhanced LRC) disorot kata demi kata. Lirik tanpa waktu per kata menyala per baris.

Lirik disimpan di folder data aplikasi (`%LOCALAPPDATA%\io.github.daendells.encore`), satu file per lagu. Hasil "tidak ditemukan" disimpan 7 hari. Kesalahan jaringan tidak disimpan, jadi lagu itu dicoba lagi saat diputar berikutnya.

### Tampilan

- Lebar strip paling besar 60% lebar layar, rata tengah. Baris yang lebih panjang dibungkus paling banyak 2 baris.
- Baris sekarang berukuran 22 px (bawaan), semibold, putih. Baris sebelumnya dan berikutnya berukuran 70% dari itu. Keduanya abu-abu pekat (#a8a8a8 dan #c8c8c8), bukan putih transparan, supaya garis tepi hitam 1 px tidak tembus pandang.
- Kontras 4,5:1 atau lebih di atas latar terang maupun gelap dicapai oleh garis tepi hitam itu, tidak oleh pelat latar saja. Pelat latarnya `rgba(0,0,0,.35)` dengan sudut 12 px.
- Lagu instrumental, intro lebih dari 5 detik, atau jeda lebih dari 20 detik antar baris ditandai dengan "♪". Kalau lirik tidak ditemukan, judul dan artis tampil 3 detik lalu strip hilang sendiri.

## Angka terukur

Semua angka diukur pada versi rilis (`npm run tauri build`) dengan satu lagu diputar dari Spotify desktop. Target yang tidak tercapai ditulis apa adanya.

| Hal | Target | Hasil | Arti |
|---|---|---|---|
| Selisih rata-rata waktu baris tampil terhadap waktu di LRC | kurang dari 150 ms | TODO-ANGKA ms | Rata-rata nilai mutlak selisih pada baris yang dimunculkan timer, dari log waktu |
| Selisih p95 | kurang dari 300 ms | TODO-ANGKA ms | 95% baris tampil dengan selisih sekecil ini atau lebih kecil |
| CPU saat lagu jalan | rata-rata kurang dari 1% | TODO-ANGKA % | Persen dari seluruh inti logis (seperti Task Manager). Persen dari satu inti: TODO-ANGKA % |
| RAM seluruh proses (private bytes) | kurang dari 80 MiB | TODO-ANGKA MiB | Jumlah Encore dan semua proses WebView2 miliknya |
| RAM seluruh proses (working set) | tanpa target | TODO-ANGKA MiB | Lebih besar karena menghitung halaman yang dipakai bersama |
| Ukuran installer | kurang dari 15 MiB | 1,75 MiB (1.839.683 byte) | `Encore_0.1.0_x64-setup.exe` |
| Salah cocok pada 50 lagu uji | 0 | TODO-ANGKA | Lagu yang mendapat lirik lagu lain |
| Lagu uji yang mendapat lirik | tanpa target | TODO-ANGKA dari 50 | Sisanya tidak ditemukan |
| Alamat tujuan jaringan selain LRCLIB | 0 | TODO-ANGKA | Alamat TCP yang terlihat selama pengukuran (TCP saja, dicuplik kira-kira tiap 350 milidetik. Koneksi singkat dan UDP/QUIC tidak terlihat) |

Tanggal ukur: TODO-ANGKA. Mesin: TODO-ANGKA.

Cara mengukur ulang:

```powershell
# waktu: jalankan Encore dengan log, putar lagu, lalu baca log
$env:ENCORE_TIMING_LOG = "$env:TEMP\encore-timing.csv"
.\src-tauri\target\release\encore.exe
node scripts\timing-report.mjs "$env:TEMP\encore-timing.csv"

# CPU, RAM, dan alamat jaringan selama 300 detik (Encore harus sedang jalan)
powershell -File scripts\measure-resources.ps1 -Seconds 300

# contoh kecil terhadap LRCLIB (set uji buta 50 lagu disimpan di luar repo)
cd src-tauri
cargo run --release --example eval -- ..\eval\sample.json hasil.json
```

Log waktu hanya aktif kalau `ENCORE_TIMING_LOG` diisi. Yang dihitung hanya baris yang dimunculkan timer, yaitu pindah tepat satu baris maju. Baris yang muncul karena jeda atau lanjut putar, tampil atau sembunyi, geser offset, seek, ganti pengaturan, atau baris pertama lagu tidak dihitung, dan skrip menyebutkan jumlahnya. Pengukuran jaringan hanya melihat koneksi TCP yang dicuplik kira-kira tiap 350 milidetik, jadi koneksi singkat dan UDP/QUIC tidak terlihat. Log ini mengukur kapan Encore menggambar baris, tidak kapan suara terdengar. Karena itu offset tetap perlu untuk keterlambatan tetap dari pemutar.

## Batasan

- Perlu WebView2 Runtime. Windows 11 dan Windows 10 yang terbaru sudah menyertakannya.
- Perlu Windows 10 versi 2004 atau lebih baru, karena strip disembunyikan dari tangkapan layar dengan `SetWindowDisplayAffinity`.
- Strip tidak ikut terlihat di berbagi layar, rekaman, dan screenshot. Ini berlaku untuk semua jenis tangkapan, tidak hanya rapat. Encore tidak mengenali aplikasi rapat. Pilihan ini bisa dimatikan di pengaturan, misalnya untuk mengambil screenshot.
- Cakupan LRCLIB terbatas. Lagu yang belum ada di sana, atau yang hanya punya lirik tanpa waktu, ditampilkan sebagai tidak ditemukan.
- Lirik di YouTube muncul kalau videonya sama panjang dengan lagu rilisannya (selisih paling banyak 2 detik). Klip musik dengan intro atau outro tambahan tidak mendapat lirik, karena Encore menolak kandidat yang durasinya tidak cocok.
- Pemutar tanpa SMTC tidak didukung. Yang diuji hanya Spotify desktop dan Chrome. Edge, Firefox, dan Spotify web belum diuji. Jalur Chrome sudah diverifikasi langsung di Chrome yang memutar YouTube.
- Chrome hanya melapor posisi saat play, pause, seek, dan ganti lagu. Encore menghitung sisanya sendiri. Pada uji awal, hasil hitungan itu berselisih 50 sampai 85 ms dari laporan berikutnya.
- Pintasan `Win+Alt+...` bisa bentrok dengan aplikasi lain. Pintasan yang bentrok tidak aktif, dan jendela pengaturan menandai pintasan yang dipakai aplikasi lain. Encore tetap jalan.
- Supaya RAM tetap di bawah 80 MiB, WebView2 untuk strip berjalan dalam mode satu proses. Microsoft tidak mendukung mode ini secara resmi. Kalau WebView2 mogok, Encore memulai ulang dirinya sendiri, paling sering sekali tiap 30 detik.
- Tiga pengaturan WebView2 yang tidak terdokumentasi mencegah dua koneksi di awal: ke Microsoft (substrate.office.com) dan ke DNS publik Google (8.8.8.8 dan 8.8.4.4, DNS lewat HTTPS). Pembaruan WebView2 bisa mengubah ini, jadi pemeriksaan jaringan perlu diulang setelah WebView2 diperbarui.
- Hanya Windows. Tidak ada macOS atau Linux.

## Pemasangan

Pasang dari `Encore_0.1.0_x64-setup.exe` (installer NSIS, per pengguna, tanpa hak administrator). Encore tidak punya pembaruan otomatis.

Untuk membangun sendiri, siapkan Node.js, Rust (toolchain MSVC), dan WebView2 Runtime:

```powershell
npm install
npm run tauri build
```

Installer ada di `src-tauri\target\release\bundle\nsis\`. Untuk menjalankan uji: masuk ke `src-tauri`, lalu `cargo test`.

## Pintasan dan tray

| Pintasan | Fungsi |
|---|---|
| `Win+Alt+L` | Tampilkan atau sembunyikan strip |
| `Win+Alt+]` | Lirik muncul 250 ms lebih awal (disimpan per lagu) |
| `Win+Alt+[` | Lirik muncul 250 ms lebih lambat (disimpan per lagu) |
| `Win+Alt+O` | Buka pengaturan |
| `Ctrl` + seret | Geser strip, posisinya diingat |

Ikon tray menyediakan tampilkan atau sembunyikan, ukuran teks (kecil 18 px, sedang 22 px, besar 28 px), kembalikan posisi ke bawah tengah, mulai saat Windows menyala, Pengaturan, dan keluar. Offset per aplikasi untuk keterlambatan tetap ada di pengaturan.

## Privasi

Satu-satunya server yang dihubungi Encore adalah `https://lrclib.net`. Encore mengirim setiap judul dan nama artis yang dilaporkan pemutar, beserta album dan durasinya. Ini termasuk video yang bukan musik dan nama channel YouTube. Saat strip disembunyikan, pencarian untuk lagu baru tetap jalan supaya liriknya siap begitu strip ditampilkan lagi. Encore tidak punya analitik, pembaruan otomatis, font daring, atau CDN. WebView2 dijalankan tanpa jaringan latar belakang dan tanpa pembaruan komponen.

## Lisensi

MIT. Lihat [LICENSE](LICENSE). Hak cipta 2026 Daendells.

---

# English

Encore shows the lyrics of the song you are playing on Windows. They appear as a three-line strip just above the taskbar, always on top, and clicks pass through it. The current line lights up in time with the song.

## How it works

1. Encore reads title, artist, album, duration, position and play state from the Windows media controls (SMTC). No login, no Spotify API. In testing, desktop Spotify and Chrome (YouTube, YouTube Music) provide this data.
2. Title and artist are cleaned of decorations such as "(Remastered 2011)", "feat." and "(Official Video)". Encore then asks [LRCLIB](https://lrclib.net) for time-synced lyrics with title, artist, album and duration.
3. If the first request fails, Encore searches and scores each candidate by title and artist similarity. The duration must differ by at most 2 seconds. If none qualifies, the lyrics count as not found. Encore is designed to reject the lyrics of another song: when in doubt, the song counts as not found.
4. The song position is the player's last report plus the time elapsed since that report. The clock resyncs on every report. A jump of more than 1.5 seconds counts as a seek.
5. One timer is set for the next line and recomputed on every sync. There is no fast polling. While the strip is hidden or nothing is playing, no timer runs and the strip window draws nothing. The SMTC listeners stay active, and new songs are still looked up so their lyrics are ready when the strip comes back.
6. The strip window (WebView2) only draws. Line changes use a 180 ms CSS transition, which is off when Windows is set to reduce motion. Lyrics with per-word timing (enhanced LRC) are highlighted word by word. Lyrics without it light up line by line.

Lyrics are cached in the app data folder (`%LOCALAPPDATA%\io.github.daendells.encore`), one file per song. "Not found" results are kept for 7 days. Network errors are not stored, so the song is retried on its next play.

### Appearance

- The strip is at most 60% of the screen width, centred. Longer lines wrap onto at most 2 lines.
- The current line is 22 px by default, semibold, white. The previous and next lines are 70% of that size in opaque grey (#a8a8a8 and #c8c8c8) instead of translucent white, so the 1 px black outline is never see-through.
- A contrast of 4.5:1 or more over both light and dark backgrounds comes from that black outline, not from the plate alone. The plate is `rgba(0,0,0,.35)` with 12 px corners.
- Instrumentals, intros longer than 5 seconds and gaps longer than 20 seconds between lines show "♪". If no lyrics are found, title and artist show for 3 seconds and the strip hides itself.

## Measured numbers

All numbers come from a release build (`npm run tauri build`) playing one song from desktop Spotify. Targets that were not met are stated as such.

| Item | Target | Result | Meaning |
|---|---|---|---|
| Mean offset between a line appearing and its LRC time | under 150 ms | TODO-ANGKA ms | Mean of the absolute offset over lines the timer put on screen, from the timing log |
| p95 offset | under 300 ms | TODO-ANGKA ms | 95% of lines appear with this offset or less |
| CPU while a song plays | mean under 1% | TODO-ANGKA % | Percent of all logical cores (as in Task Manager). Percent of one core: TODO-ANGKA % |
| RAM of all processes (private bytes) | under 80 MiB | TODO-ANGKA MiB | Encore plus all its WebView2 processes |
| RAM of all processes (working set) | no target | TODO-ANGKA MiB | Larger because it counts shared pages |
| Installer size | under 15 MiB | 1.75 MiB (1,839,683 bytes) | `Encore_0.1.0_x64-setup.exe` |
| Wrong matches in the 50-song test set | 0 | TODO-ANGKA | Songs that received another song's lyrics |
| Test songs that got lyrics | no target | TODO-ANGKA of 50 | The rest were not found |
| Network destinations other than LRCLIB | 0 | TODO-ANGKA | TCP addresses seen during the measurement (TCP only, sampled about every 350 ms. Short connections and UDP/QUIC are not seen) |

Measured on: TODO-ANGKA. Machine: TODO-ANGKA.

To measure again, use the PowerShell commands in the Indonesian section above (the 50-song blind set is kept outside the repo, `eval/sample.json` is a small example). The timing log counts only lines the timer put on screen, that is, a move forward by exactly one line. Lines shown after pause or resume, show or hide, an offset nudge, a seek, a settings change or the first line of a song are not counted, and the script reports how many were left out. The network check sees TCP connections only, sampled about every 350 ms, so short connections and UDP/QUIC are not seen. The timing log records when Encore draws a line, not when the sound is heard, so the per-app offset stays useful for fixed player delay.

## Limitations

- Needs the WebView2 Runtime (included in Windows 11 and up-to-date Windows 10).
- Needs Windows 10 version 2004 or newer, because the strip is hidden from captures with `SetWindowDisplayAffinity`.
- The strip does not appear in screen sharing, recordings or screenshots. This applies to every kind of capture, not only meetings. Encore does not detect meeting apps. You can turn it off in settings, for example to take a screenshot.
- LRCLIB coverage is limited. Songs missing there, or with only unsynced lyrics, show as not found.
- Lyrics on YouTube appear when the video is as long as the released track (within 2 seconds). Music videos with an added intro or outro get no lyrics, because Encore rejects candidates with a non-matching duration.
- Players without SMTC are not supported. Only desktop Spotify and Chrome were tested. Edge, Firefox and web Spotify were not. The Chrome path was verified live in Chrome playing YouTube.
- Chrome reports position only on play, pause, seek and track change. Encore computes the rest. In the early test, that computed position differed by 50 to 85 ms from the next report.
- `Win+Alt+...` shortcuts can clash with other apps. A clashing shortcut stays inactive, and the settings window marks shortcuts taken by another app. Encore keeps running.
- To stay under 80 MiB of RAM, the strip runs WebView2 in single-process mode, which Microsoft does not officially support. If WebView2 crashes, Encore restarts itself, at most once every 30 seconds.
- Three undocumented WebView2 feature switches stop two connections at start: one to Microsoft (substrate.office.com) and one to Google public DNS (8.8.8.8 and 8.8.4.4, DNS over HTTPS). A WebView2 update may change this, so repeat the network check after updates.
- Windows only. No macOS or Linux.

## Install and build

Install from `Encore_0.1.0_x64-setup.exe` (NSIS, per user, no administrator rights). Encore has no auto-update.

To build, you need Node.js, Rust (MSVC toolchain) and the WebView2 Runtime:

```powershell
npm install
npm run tauri build
```

The installer is in `src-tauri\target\release\bundle\nsis\`. To run the tests, go to `src-tauri` and run `cargo test`.

## Shortcuts and tray

| Shortcut | Action |
|---|---|
| `Win+Alt+L` | Show or hide the strip |
| `Win+Alt+]` | Lyrics appear 250 ms earlier (saved per song) |
| `Win+Alt+[` | Lyrics appear 250 ms later (saved per song) |
| `Win+Alt+O` | Open settings |
| `Ctrl` + drag | Move the strip, the position is remembered |

The tray icon offers show or hide, text size (small 18 px, medium 22 px, large 28 px), reset position to bottom centre, start with Windows, Settings, and quit. A per-app offset for fixed delay is in settings.

## Privacy

The only server Encore contacts is `https://lrclib.net`. It receives every media title and artist the player reports, with the album and duration. That includes videos that are not music and YouTube channel names. While the strip is hidden, lookups for new songs still run so the lyrics are ready when it comes back. There is no analytics, auto-update, web font or CDN. WebView2 runs with background networking and component updates turned off.

## Licence

MIT. See [LICENSE](LICENSE). Copyright 2026 Daendells.
