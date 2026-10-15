# ytb2pdf — YouTube → PDF

Desktop app (macOS trước) biến video YouTube dài — bài giảng, tutorial nhiều hình ảnh —
thành tài liệu PDF: mỗi đoạn caption đi kèm frame hình đúng thời điểm đó, loại bỏ ảnh
trùng, tuỳ chọn dịch caption (VD: Anh → Việt).

```
YouTube URL → metadata → caption → (dịch) → tải video 720p
            → chụp frame tại timestamp mỗi caption → loại frame trùng (dHash)
            → dựng PDF (bìa + [timestamp | ảnh | caption gốc | bản dịch])
```

## Kiến trúc

Học theo [FilmCraft](https://github.com/storytold/filmcraft): workspace nhiều lớp,
engine tách hẳn khỏi UI.

```
crates/
  ytb2pdf-core/   engine, không biết gì về UI
    youtube.rs    parse URL + gọi yt-dlp (metadata, caption, tải video)
    subtitles.rs  parse SRT/VTT/JSON3, chuẩn hoá caption
    frames.rs     ffmpeg chụp frame + dHash loại ảnh trùng
    translate.rs  trait Translator; GeminiTranslator (batch, pace theo free-tier)
    pdf.rs        dựng PDF bằng genpdf (pure Rust)
    pipeline.rs   chạy các stage theo thứ tự, báo progress qua callback
  ytb2pdf-cli/    CLI: ytb2pdf <url> [--to vi] [--cookies-from-browser chrome]
  ytb2pdf-app/    app desktop native bằng egui (chạy pipeline trên worker thread)
```

**Không link FFmpeg/yt-dlp vào binary** — gọi qua CLI ngoài. yt-dlp bám YouTube sát hơn
bất kỳ lib Rust nào; ffmpeg thì khỏi bàn.

## Yêu cầu

- Rust stable (đã test với 1.99)
- `yt-dlp` và `ffmpeg` có trên PATH
- Dịch caption: `GEMINI_API_KEY` (key free từ aistudio.google.com)

## Chạy thử

```sh
# CLI
cargo run -p ytb2pdf-cli -- "https://youtu.be/W25_BbZCZD0" --to vi

# V2 mode: tiết kiệm bandwidth cho video dài (detect scene ở 240p, chỉ tải đoạn cần ở 720p)
cargo run -p ytb2pdf-cli -- "https://youtu.be/..." --v2 --cookies cookies.txt

# Text-only: PDF nhẹ cho mobile (chỉ caption, không ảnh)
cargo run -p ytb2pdf-cli -- "https://youtu.be/..." --text-only --cookies cookies.txt

# App desktop
cargo run -p ytb2pdf-app
```

App lưu PDF ra `~/Desktop/ytb2pdf-<id>.pdf`, file tạm ở `~/.ytb2pdf/`.

> Gặp HTTP 403 / "Sign in to confirm you're not a bot" (mạng nhà hay bị):
> điền tên browser vào ô "Cookie từ browser" (vd `chrome`) để yt-dlp mượn cookie
> đăng nhập YouTube của anh.

## Đóng gói macOS

`cargo build` trên Linux **không** ra được binary macOS. Cách làm (như FilmCraft):
GitHub Actions chạy trên `macos-14` (Apple Silicon), build release rồi gói `.dmg`
bằng `hdiutil` — xem `.github/workflows/macos.yml`. Muốn ký + notarize thì thêm
Apple Developer certificate vào repo secrets (đang để TODO).

## TODO

- [ ] Nhúng font Việt mặc định vào binary (hiện tại tự tìm DejaVu/Arial trên máy)
- [ ] Chọn frame thông minh hơn: scene-change detection thay vì midpoint mỗi caption
- [ ] Whisper fallback khi video không có caption (tái dùng pipeline mini-youglish)
- [ ] Ký + notarize .dmg (cần Apple Developer cert)
- [ ] Đặt tên app chính thức (hiện tại là `ytb2pdf` tạm)
