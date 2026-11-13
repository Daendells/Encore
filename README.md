# Encore

Encore menampilkan lirik lagu yang sedang Anda putar di Windows. Liriknya muncul sebagai strip tiga baris tepat di atas taskbar, selalu di depan jendela lain, dan klik menembusnya. Baris yang sedang dinyanyikan menyala sesuai waktu lagu.

![Strip lirik Encore di atas taskbar](docs/screenshot.png)

Kode dan unduhan ada di https://github.com/Daendells/Encore. Bahasa Indonesia ada di bawah ini, [English version below](#english).

Penjelasan semua fitur ada di [docs/fitur.md](docs/fitur.md), panduan pengembangan (bahasa Inggris) di [docs/development.md](docs/development.md).

## Cara kerja

1. Encore membaca judul, artis, album, durasi, posisi, dan status putar dari kontrol media Windows (SMTC). Tidak ada login dan tidak memakai API Spotify. Pada uji, Spotify desktop dan Chrome (YouTube, YouTube Music) memberi data ini.
2. Judul dan artis dibersihkan dari hiasan seperti "(Remastered 2011)", "feat.", dan "(Official Video)". Encore lalu meminta lirik bertimbang waktu ke [LRCLIB](https://lrclib.net) dengan judul, artis, album, dan durasi.
3. Kalau permintaan pertama gagal, Encore mencari lalu memberi skor pada setiap kandidat dari kemiripan judul dan artis. Durasinya harus berselisih paling banyak 2 detik. Kalau tidak ada yang lolos, lirik dianggap tidak ditemukan. Encore dirancang untuk menolak lirik lagu lain: kalau ragu, lagunya dianggap tidak ditemukan.
4. Posisi lagu dihitung dari laporan terakhir pemutar ditambah waktu yang berlalu sejak laporan itu. Jam ini disinkronkan ulang setiap kali pemutar melapor. Lompatan lebih dari 1,5 detik dianggap seek.
5. Satu timer dibuat untuk baris berikutnya dan dihitung ulang setiap sinkronisasi. Tidak ada polling cepat. Saat strip disembunyikan atau tidak ada lagu, tidak ada timer yang jalan dan jendela strip tidak menggambar. Pendengar SMTC tetap aktif, dan lagu baru tetap dicari liriknya supaya siap begitu strip ditampilkan lagi.
6. Jendela strip (WebView2) hanya menggambar. Pergantian baris memakai transisi CSS 180 ms, dan transisi dimatikan kalau Windows diatur untuk mengurangi gerakan. Lirik yang punya waktu per kata (enhanced LRC) disorot kata demi kata. Lirik tanpa waktu per kata menyala per baris.

Lirik disimpan di folder data aplikasi (`%LOCALAPPDATA%\io.github.daendells.encore`), satu file per lagu. Hasil "tidak ditemukan" disimpan 7 hari. Kesalahan jaringan tidak disimpan, jadi lagu itu dicoba lagi saat diputar berikutnya. Kalau internet putus, Encore mencari lagi liriknya saat koneksi kembali.

### Tampilan

- Lebar strip bawaannya 60% lebar layar (bisa 40% sampai 90%), rata tengah. Baris yang lebih panjang dibungkus paling banyak 2 baris.
- Pengaturan tampilan: posisi (bawah tengah, atas tengah, kiri bawah, kanan bawah), satu atau tiga baris, ukuran teks, latar gelap 0% sampai 70%, lebar, dan warna baris yang sedang dinyanyikan (putih, kuning, hijau, biru). Empat gaya siap pakai mengatur semuanya sekaligus: Bawaan, Minimalis (1 baris, tanpa latar), Karaoke (30 px, latar 55%, lebar 80%, kuning), dan Fokus (atas tengah, 1 baris, biru). Mengubah satu nilai membuat gayanya "Kustom". Rinciannya ada di [docs/fitur.md](docs/fitur.md).
- Baris sekarang berukuran 22 px (bawaan), semibold, putih. Baris sebelumnya dan berikutnya berukuran 70% dari itu. Keduanya abu-abu pekat (#a8a8a8 dan #c8c8c8), bukan putih transparan, supaya garis tepi hitam 1 px tidak tembus pandang.
- Kontras 4,5:1 atau lebih di atas latar terang maupun gelap dicapai oleh garis tepi hitam itu, tidak oleh pelat latar saja. Pelat latarnya `rgba(0,0,0,.35)` dengan sudut 12 px.
- Lagu instrumental, intro lebih dari 5 detik, atau jeda lebih dari 20 detik antar baris ditandai dengan "♪". Kalau lirik tidak ditemukan, judul dan artis tampil 3 detik lalu strip hilang sendiri.

## Angka terukur

Semua angka diukur pada versi rilis (commit 3d30e28) di laptop Windows 10 Home 19045 dengan WebView2 157, pada 13 November 2026. Perbaikan DNS sesudahnya tidak memengaruhi angka-angka ini. Target yang tidak tercapai ditulis apa adanya.

| Hal | Target | Hasil | Arti |
|---|---|---|---|
| Keterlambatan baris tampil setelah waktu di LRC, rata-rata | kurang dari 150 ms | 10,9 ms | 101 pergantian baris pada 5 lagu Spotify. Diukur terhadap posisi yang dilaporkan Spotify ke Windows, bukan terhadap suara |
| Keterlambatan baris tampil, p95 | kurang dari 300 ms | 17 ms | 95% baris tampil dengan keterlambatan sekecil ini atau lebih kecil. Terlama 19 ms |
| CPU saat lagu jalan | rata-rata kurang dari 1% | 0,10% | Persen dari seluruh inti, rata-rata. Setara 1,21% dari satu inti |
| RAM saat lagu jalan dan lirik tampil (private bytes) | kurang dari 80 MiB | 64,2 MiB | Jumlah `encore.exe` dan proses WebView2-nya, rata-rata selama 120 detik. Tertinggi 66,5 MiB. Saat strip disembunyikan: 61,7 MiB |
| RAM saat lagu jalan (working set) | tanpa target | sekitar 225 MiB | Jauh lebih besar karena menghitung halaman WebView2 yang dipakai bersama. Saat strip disembunyikan: sekitar 71 MiB |
| Ukuran installer | kurang dari 15 MiB | 1,75 MiB (1.839.946 byte) | `Encore_0.1.0_x64-setup.exe` |
| Lirik salah pada 50 lagu uji | 0 | 0 | Tidak ada lagu yang mendapat lirik lagu lain |
| Lagu uji yang tidak mendapat lirik padahal ada | 0 | 0 | 11 dari 50 lagu memang harus tanpa lirik |
| Alamat tujuan jaringan selain LRCLIB | 0 | 0 | Dalam uji 120 detik dengan satu lagu baru, hanya `lrclib.net` yang dihubungi, oleh `encore.exe`. Proses WebView2 tidak membuat koneksi (lihat catatan di bawah) |

Pencocokan lirik diuji dengan 50 lagu nyata yang disusun oleh orang yang tidak melihat kode pencocokan: lagu Indonesia, Inggris, Jepang, Korea, dan bahasa lain, dengan judul berhiasan (remaster, live, feat., judul YouTube). Sebelas lagu di antaranya memang tidak boleh mendapat lirik. Uji dijalankan satu kali. Pemeriksa judul yang ketat melaporkan 44 lagu benar dan 6 lagu perlu dilihat. Setelah 6 lagu itu diperiksa lewat isi liriknya, 5 ternyata lirik lagu yang sama dengan judul tersimpan lebih panjang (versi live, judul YouTube, atau "movie ver."). Satu lagu adalah instrumental yang tampil sebagai "♪".


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

Log waktu hanya aktif kalau `ENCORE_TIMING_LOG` diisi. Yang dihitung hanya baris yang dimunculkan timer, yaitu pindah tepat satu baris maju. Baris yang muncul karena jeda atau lanjut putar, tampil atau sembunyi, geser offset, seek, ganti pengaturan, atau baris pertama lagu tidak dihitung, dan skrip menyebutkan jumlahnya. Pengukuran jaringan hanya melihat koneksi TCP yang dicuplik kira-kira tiap 450 milidetik, jadi koneksi singkat dan UDP/QUIC tidak terlihat. Log ini membandingkan waktu baris tampil dengan posisi yang dilaporkan pemutar ke Windows, bukan dengan suara yang terdengar. Karena itu offset tetap perlu untuk keterlambatan tetap dari pemutar.

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

Ikon tray menyediakan tampilkan atau sembunyikan, gaya tampilan (Bawaan, Minimalis, Karaoke, Fokus), ukuran teks (kecil 18 px, sedang 22 px, besar 28 px), posisi (bawah tengah, atas tengah, kiri bawah, kanan bawah), kembalikan posisi ke bawah tengah, mulai saat Windows menyala, Pengaturan, dan keluar. Offset per aplikasi untuk keterlambatan tetap ada di pengaturan.

## Privasi

Satu-satunya server yang dihubungi Encore adalah `https://lrclib.net`. Encore mengirim setiap judul dan nama artis yang dilaporkan pemutar, beserta album dan durasinya. Ini termasuk video yang bukan musik dan nama channel YouTube. Saat strip disembunyikan, pencarian untuk lagu baru tetap jalan supaya liriknya siap begitu strip ditampilkan lagi. Encore tidak punya analitik, pembaruan otomatis, font daring, atau CDN. WebView2 dijalankan tanpa jaringan latar belakang dan tanpa pembaruan komponen.

## Lisensi

MIT. Lihat [LICENSE](LICENSE). Hak cipta 2026 Daendells.

---

# English

Encore shows the lyrics of the song you are playing on Windows. They appear as a three-line strip just above the taskbar, always on top, and clicks pass through it. The current line lights up in time with the song.

Code and downloads: https://github.com/Daendells/Encore

A full feature description (in Indonesian) is in [docs/fitur.md](docs/fitur.md), and the development guide is in [docs/development.md](docs/development.md).

## How it works

1. Encore reads title, artist, album, duration, position and play state from the Windows media controls (SMTC). No login, no Spotify API. In testing, desktop Spotify and Chrome (YouTube, YouTube Music) provide this data.
2. Title and artist are cleaned of decorations such as "(Remastered 2011)", "feat." and "(Official Video)". Encore then asks [LRCLIB](https://lrclib.net) for time-synced lyrics with title, artist, album and duration.
3. If the first request fails, Encore searches and scores each candidate by title and artist similarity. The duration must differ by at most 2 seconds. If none qualifies, the lyrics count as not found. Encore is designed to reject the lyrics of another song: when in doubt, the song counts as not found.
4. The song position is the player's last report plus the time elapsed since that report. The clock resyncs on every report. A jump of more than 1.5 seconds counts as a seek.
5. One timer is set for the next line and recomputed on every sync. There is no fast polling. While the strip is hidden or nothing is playing, no timer runs and the strip window draws nothing. The SMTC listeners stay active, and new songs are still looked up so their lyrics are ready when the strip comes back.
6. The strip window (WebView2) only draws. Line changes use a 180 ms CSS transition, which is off when Windows is set to reduce motion. Lyrics with per-word timing (enhanced LRC) are highlighted word by word. Lyrics without it light up line by line.

Lyrics are cached in the app data folder (`%LOCALAPPDATA%\io.github.daendells.encore`), one file per song. "Not found" results are kept for 7 days. Network errors are not stored, so the song is retried on its next play. If the internet drops, Encore looks up the song again when the connection returns.

### Appearance

- The strip is 60% of the screen width by default (40% to 90%), centred. Longer lines wrap onto at most 2 lines.
- Display settings: position (bottom centre, top centre, bottom left, bottom right), one or three lines, text size, dark background 0% to 70%, width, and the colour of the line being sung (white, yellow, green, blue). Four ready-made styles set everything at once: Default, Minimal (1 line, no background), Karaoke (30 px, 55% background, 80% wide, yellow) and Focus (top centre, 1 line, blue). Changing any one value makes the style Custom. Details are in [docs/fitur.md](docs/fitur.md) (Indonesian).
- The current line is 22 px by default, semibold, white. The previous and next lines are 70% of that size in opaque grey (#a8a8a8 and #c8c8c8) instead of translucent white, so the 1 px black outline is never see-through.
- A contrast of 4.5:1 or more over both light and dark backgrounds comes from that black outline, not from the plate alone. The plate is `rgba(0,0,0,.35)` with 12 px corners.
- Instrumentals, intros longer than 5 seconds and gaps longer than 20 seconds between lines show "♪". If no lyrics are found, title and artist show for 3 seconds and the strip hides itself.

## Measured numbers

All numbers come from a release build (commit 3d30e28) on a Windows 10 Home 19045 laptop with WebView2 157, on 13 November 2026. The DNS fix made afterwards does not affect them. Targets that were not met are stated as such.

| Item | Target | Result | Meaning |
|---|---|---|---|
| Delay of a line appearing after its LRC time, mean | under 150 ms | 10.9 ms | 101 line changes over 5 Spotify songs. Measured against the position Spotify reports to Windows, not against the audio |
| Delay of a line appearing, p95 | under 300 ms | 17 ms | 95% of lines appear with this delay or less. The longest was 19 ms |
| CPU while a song plays | mean under 1% | 0.10% | Percent of all cores, on average. This equals 1.21% of one core |
| RAM while a song plays and lyrics show (private bytes) | under 80 MiB | 64.2 MiB | `encore.exe` plus its WebView2 process, averaged over 120 seconds. Peak 66.5 MiB. With the strip hidden: 61.7 MiB |
| RAM while a song plays (working set) | no target | about 225 MiB | Much larger because it counts WebView2 pages shared between processes. With the strip hidden: about 71 MiB |
| Installer size | under 15 MiB | 1.75 MiB (1,839,946 bytes) | `Encore_0.1.0_x64-setup.exe` |
| Wrong lyrics in the 50-song test set | 0 | 0 | No song received another song's lyrics |
| Test songs that got no lyrics although they exist | 0 | 0 | 11 of the 50 songs must get no lyrics |
| Network destinations other than LRCLIB | 0 | 0 | In a 120 second run with a new song, only `lrclib.net` was contacted, by `encore.exe`. The WebView2 process made no connection (see the note below) |

Lyric matching was tested with 50 real songs chosen by someone who did not see the matching code: Indonesian, English, Japanese, Korean and other languages, with decorated titles (remaster, live, feat., YouTube titles). Eleven of them must get no lyrics. The test was run once. The strict title check reported 44 songs correct and 6 to inspect. After checking those 6 by their lyric text, 5 were the same song's lyrics stored under a longer title (live, YouTube or "movie ver."). One was an instrumental shown as "♪".


To measure again, use the PowerShell commands in the Indonesian section above (the 50-song blind set is kept outside the repo, `eval/sample.json` is a small example). The timing log counts only lines the timer put on screen, that is, a move forward by exactly one line. Lines shown after pause or resume, show or hide, an offset nudge, a seek, a settings change or the first line of a song are not counted, and the script reports how many were left out. The network check sees TCP connections only, sampled about every 450 ms, so short connections and UDP/QUIC are not seen. The timing log compares when a line appears with the position the player reports to Windows, not with the sound you hear, so the per-app offset stays useful for fixed player delay.

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

The tray icon offers show or hide, display style (Default, Minimal, Karaoke, Focus), text size (small 18 px, medium 22 px, large 28 px), position (bottom centre, top centre, bottom left, bottom right), reset position to bottom centre, start with Windows, Settings, and quit. A per-app offset for fixed delay is in settings.

## Privacy

The only server Encore contacts is `https://lrclib.net`. It receives every media title and artist the player reports, with the album and duration. That includes videos that are not music and YouTube channel names. While the strip is hidden, lookups for new songs still run so the lyrics are ready when it comes back. There is no analytics, auto-update, web font or CDN. WebView2 runs with background networking and component updates turned off.

## Licence

MIT. See [LICENSE](LICENSE). Copyright 2026 Daendells.
