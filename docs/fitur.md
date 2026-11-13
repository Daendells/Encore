# Fitur Encore

Encore menampilkan lirik lagu yang sedang Anda putar di Windows. Liriknya muncul di atas taskbar, sehingga Anda bisa membacanya tanpa pindah jendela.

## Masalah yang diselesaikan

Lirik biasanya ada di jendela pemutar atau di situs lain. Untuk membacanya, Anda harus membuka jendela itu dan meninggalkan pekerjaan Anda. Encore menaruh lirik di strip kecil di atas taskbar. Klik menembus strip itu, jadi Anda tetap bisa bekerja di jendela di belakangnya.

## Tampilan strip

Strip menampilkan tiga baris: baris sebelumnya, baris yang sedang dinyanyikan, dan baris berikutnya.

- Lebar strip paling besar 60% lebar layar, rata tengah. Baris yang lebih panjang dibungkus paling banyak 2 baris.
- Baris sekarang berukuran 22 px (bawaan), tebal, dan putih. Baris sebelum dan sesudahnya berukuran 70% dari itu dan berwarna abu-abu.
- Pergantian baris memakai gerakan singkat (180 ms). Gerakan ini mati kalau Windows diatur untuk mengurangi animasi.

### Sorot per kata

Sebagian lirik di LRCLIB punya waktu untuk tiap kata. Pada lirik seperti ini, kata menyala satu per satu mengikuti nyanyian. Lirik yang hanya punya waktu per baris menyala per baris.

### Tanda ♪

Strip menampilkan "♪" dalam tiga keadaan:

- lagunya instrumental,
- intro lebih dari 5 detik sebelum baris pertama,
- jeda lebih dari 20 detik di antara dua baris.

### Kartu "tidak ditemukan"

Kalau lirik tidak ada, strip menampilkan judul dan artis selama 3 detik, lalu hilang sendiri. Dengan begitu Anda tahu Encore sudah membaca lagunya, tetapi tidak punya liriknya.

Kalau internet putus, Encore mencari lagi liriknya saat koneksi kembali.

## Pintasan keyboard

| Pintasan | Fungsi |
|---|---|
| `Win+Alt+L` | Menampilkan atau menyembunyikan strip |
| `Win+Alt+]` | Lirik muncul 250 ms lebih awal |
| `Win+Alt+[` | Lirik muncul 250 ms lebih lambat |
| `Win+Alt+O` | Membuka jendela pengaturan |
| `Ctrl` + seret | Memindahkan strip |

Pintasan bisa bentrok dengan aplikasi lain. Pintasan yang bentrok tidak aktif, dan jendela pengaturan menandainya. Encore tetap jalan.

## Klik menembus dan memindahkan strip

Secara bawaan klik menembus strip, dan strip tidak mengambil fokus dari jendela yang sedang Anda pakai. Tahan `Ctrl` untuk memindahkan strip. Selama `Ctrl` ditahan, strip bisa diklik dan diseret. Setelah `Ctrl` dilepas, klik menembus lagi dan posisi baru disimpan. Kalau posisi tersimpan berada di luar semua monitor (misalnya monitor sudah dicabut), strip dipindah ke monitor terdekat supaya tetap terjangkau.

Untuk mengembalikan strip ke bawah tengah, pakai menu tray.

## Menu tray

Klik kiri ikon tray menampilkan atau menyembunyikan strip. Klik kanan membuka menu:

- Tampilkan atau sembunyikan lirik
- Ukuran teks: kecil (18 px), sedang (22 px), besar (28 px)
- Kembalikan posisi ke bawah tengah
- Mulai saat Windows menyala
- Pengaturan
- Keluar

## Jendela pengaturan

Buka dengan `Win+Alt+O` atau dari tray. Isinya:

- **Ukuran teks**, dari 12 sampai 48 px.
- **Geser waktu lirik per aplikasi**, dari -2.000 sampai 2.000 ms. Daftarnya berisi pemutar yang terdeteksi sejak Encore dijalankan, ditambah pemutar yang sudah punya geseran tersimpan. Angka positif membuat lirik muncul lebih awal.
- **Sembunyikan lirik dari berbagi layar dan tangkapan layar.**
- **Mulai saat Windows menyala.**
- Daftar pintasan dan versi aplikasi.

## Geser waktu lirik

Ada dua geseran, dan keduanya dijumlahkan.

- **Per aplikasi**: untuk keterlambatan tetap dari satu pemutar. Diatur di jendela pengaturan.
- **Per lagu**: untuk satu lagu yang liriknya tidak pas. Diatur dengan `Win+Alt+]` dan `Win+Alt+[`, masing-masing 250 ms. Strip menampilkan total geseran lagu itu, misalnya "+250 ms". Geseran disimpan dan dipakai lagi saat lagu diputar berikutnya. Kalau totalnya kembali ke 0, simpanannya dihapus.

## Disembunyikan dari berbagi layar

Secara bawaan, strip tidak ikut terlihat di berbagi layar, rekaman, dan tangkapan layar. Strip tetap terlihat di layar Anda sendiri. Ini berlaku untuk semua jenis tangkapan, karena Encore tidak mengenali aplikasi rapat. Pilihan ini bisa dimatikan di pengaturan, misalnya untuk mengambil screenshot. Fitur ini perlu Windows 10 versi 2004 atau lebih baru. Di versi yang lebih lama, strip ikut tertangkap.

## Bahasa

Teks tray dan pengaturan mengikuti bahasa tampilan Windows. Kalau Windows memakai bahasa Indonesia, Encore memakai bahasa Indonesia. Untuk bahasa lain, Encore memakai bahasa Inggris. Lirik tidak diterjemahkan.

## Mulai saat Windows menyala

Aktifkan dari tray atau jendela pengaturan. Tanda centang di tray mengikuti keadaan sebenarnya di Windows, jadi tetap benar kalau diubah dari jendela pengaturan.

## Pemutar yang didukung

Encore membaca lagu dari kontrol media Windows (SMTC), yaitu yang juga menampilkan judul lagu saat Anda mengubah volume. Tidak perlu login, dan Encore tidak memakai API Spotify.

| Pemutar | Status |
|---|---|
| Spotify desktop | Diuji |
| YouTube Music di Chrome | Diuji |
| YouTube di Chrome | Diuji. Lirik muncul kalau durasi video sama dengan lagu rilisan, selisih paling besar 2 detik |
| Edge, Firefox, Spotify web | Belum diuji |
| Pemutar tanpa SMTC | Tidak didukung |

Pemutar lain yang memakai SMTC secara teori ikut terbaca, tetapi belum diuji.

Batasan lain:

- Klip musik dengan intro atau outro tambahan tidak mendapat lirik, karena durasinya tidak cocok.
- Kalau beberapa pemutar aktif, Encore mengikuti yang sedang memutar.
- Chrome hanya melapor posisi saat putar, jeda, seek, dan ganti lagu. Encore menghitung sisanya sendiri.

## Cara lirik dicari

1. Judul dan artis dibersihkan dari hiasan seperti "(Remastered 2011)", "feat.", dan "(Official Video)".
2. Encore meminta lirik ke [LRCLIB](https://lrclib.net) dengan judul, artis, album, dan durasi.
3. Kalau permintaan itu tidak menghasilkan kecocokan, Encore mencari di LRCLIB dan menilai tiap kandidat. Syaratnya: judul cukup mirip, artis cukup mirip, dan durasi berselisih paling banyak 2 detik.
4. Kalau tidak ada kandidat yang lolos, lagunya dianggap tidak ditemukan.

### Kenapa lebih baik tidak menampilkan apa-apa

Lirik lagu lain lebih mengganggu daripada strip kosong. Karena itu, kalau ragu, Encore menganggap lagunya tidak ditemukan. Pada uji dengan 50 lagu, tidak ada lagu yang mendapat lirik lagu lain.

Lirik yang tidak punya waktu juga tidak ditampilkan, karena tidak bisa mengikuti lagu.

## Cara waktu dihitung

Pemutar hanya sesekali melapor posisi lagu ke Windows. Encore mengingat laporan terakhir, lalu menambahkan waktu yang berlalu sejak laporan itu. Jam ini disesuaikan lagi setiap kali pemutar melapor. Lompatan lebih dari 1,5 detik dianggap seek, dan strip langsung pindah ke baris yang pas.

Encore memasang satu timer untuk baris berikutnya, jadi tidak ada pengecekan terus-menerus. Saat strip disembunyikan atau tidak ada lagu, tidak ada timer yang jalan.

Pada uji dengan 5 lagu Spotify, baris tampil rata-rata 10,9 ms setelah waktunya. Angka ini dibandingkan dengan posisi yang dilaporkan Spotify, bukan dengan suara yang terdengar. Kalau suara dan lirik tetap tidak pas, pakai geseran waktu.

## Privasi

Satu-satunya server yang dihubungi Encore adalah `https://lrclib.net`. Yang dikirim adalah judul, artis, album, dan durasi dari pemutar. Ini termasuk video YouTube yang bukan musik, beserta nama channelnya, karena Encore tidak bisa membedakannya. Saat strip disembunyikan, pencarian untuk lagu baru tetap jalan supaya liriknya siap begitu strip tampil lagi.

Encore tidak punya analitik, pembaruan otomatis, font daring, atau CDN. Pemantau `Ctrl` hanya memeriksa apakah tombol itu ditekan atau dilepas, tidak ada tombol lain yang dibaca atau disimpan.

## Data yang disimpan

Semuanya ada di `%LOCALAPPDATA%\io.github.daendells.encore`:

| Berkas | Isi |
|---|---|
| `settings.json` | Ukuran teks, strip tampil atau tidak, sembunyikan dari tangkapan, posisi, geseran per aplikasi |
| `offsets.json` | Geseran per lagu |
| `cache\` | Lirik, satu berkas per lagu. Hasil "tidak ditemukan" disimpan 7 hari. Kesalahan jaringan tidak disimpan, jadi lagu itu dicoba lagi saat diputar berikutnya |

Menghapus folder ini mengembalikan Encore ke keadaan awal.

## Kalau WebView2 mogok

Strip digambar oleh WebView2. Kalau WebView2 mogok, Encore memulai ulang dirinya sendiri, paling sering sekali tiap 30 detik.
