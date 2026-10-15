# Architecture V2 Plan — Low-res Detection, High-res Extraction

**Status:** ✅ Implemented & Tested (2026-10-15)
**Author:** Daisy (với input kiến trúc từ Thái)
**Validation:** ✅ Core mechanism đã test thành công trên video thật
**Implementation:** ✅ Phase 1-3 hoàn thành, đã push lên GitHub main

---

## 1. Vấn đề (Problem)

Pipeline V1 download toàn bộ video ở 480p/720p trước khi chụp frame:

| Video | Dung lượng 720p | Thời gian download (bị throttle) |
|-------|----------------|----------------------------------|
| 30 phút | ~300MB | 10-20 phút |
| 3 giờ | ~1.2GB | 1-2 giờ |
| 12 giờ (CS50 AI) | ~2GB | 2-4 giờ |

Trong khi đó, PDF cuối cùng chỉ cần **~100-500 frame** (sau scene compacting).
90%+ dung lượng download là lãng phí.

## 2. Ý tưởng cốt lõi

**Tách video dùng để phân tích khỏi video dùng để xuất ảnh:**

1. Download bản **240p** (~10% dung lượng) để phát hiện scene.
2. Phát hiện scene change bằng FFmpeg trên bản proxy.
3. Chỉ download những **đoạn ngắn** chứa scene quan trọng ở **1080p**.
4. Trích frame chất lượng cao từ các đoạn đó.

## 3. Kết quả validate (2026-10-15)

Test trên video thật (`nLH1lNS9qD4`, 30 phút):

```sh
yt-dlp -f "bv*[height<=720]" \
  --download-sections "*60-65" \
  --force-keyframes-at-cuts \
  -o "section-test.%(ext)s" \
  "https://www.youtube.com/watch?v=nLH1lNS9qD4"
```

| Metric | Kết quả |
|--------|---------|
| Dung lượng tải | **359KB** (cho đoạn 5 giây) |
| Thời gian | 14s (bị throttle nhưng thành công) |
| Frame trích xuất | 1280×720, chất lượng tốt |
| Kết luận | ✅ `--download-sections` **thật sự tiết kiệm bandwidth** |

## 4. Kiến trúc đề xuất

```
┌─────────────┐
│ 1. PROXY    │  yt-dlp -f "bv*[height<=240]" → proxy.mp4 (~50-200MB)
│   DOWNLOAD  │
└──────┬──────┘
       │
┌──────▼──────┐
│ 2. SCENE    │  ffmpeg -vf "select='gt(scene,0.30)',showinfo" → scenes.json
│   DETECTION │  + periodic sampling mỗi 5s (bắt slide thay đổi từ từ)
└──────┬──────┘
       │
┌──────▼──────┐
│ 3. SEGMENT  │  Gộp timestamp gần nhau thành khoảng (ví dụ: 10-18s, 93-100s)
│   PLANNING  │  → segments.json
└──────┬──────┘
       │
┌──────▼──────┐
│ 4. HIGH-RES │  yt-dlp --download-sections (song song, --jobs N)
│   DOWNLOAD  │  → segments/seg_001.mp4, seg_002.mp4, ...
└──────┬──────┘
       │
┌──────▼──────┐
│ 5. FRAME    │  ffmpeg -ss <offset> -i seg.mp4 -frames:v 1 → frames/
│   EXTRACTION│  (song song, resumable như V1)
└──────┬──────┘
       │
┌──────▼──────┐
│ 6. CAPTION  │  Map mỗi caption segment → scene frame gần nhất
│   MAPPING   │  (dùng timestamp, không cần dHash ở bước này)
└──────┬──────┘
       │
┌──────▼──────┐
│ 7. PDF      │  Reuse pipeline V1: scene compacting + PDF generation
│   GENERATION│
└─────────────┘
```

## 5. Quyết định thiết kế chính

### 5.1. V2 là mode riêng, không thay thế V1

- Thêm flag `--v2` (hoặc `--smart-frames`).
- V1 (download full + caption-driven) vẫn là default — đơn giản, đáng tin cậy.
- V2 dùng cho video dài (>1h) khi download là bottleneck.

**Lý do:** V2 phức tạp hơn, có nhiều điểm failure hơn. Không nên ép mọi user dùng.

### 5.2. Scene detection: kết hợp 2 cơ chế

Như Thái đã chỉ ra, chỉ dùng `scene` score sẽ bỏ sót:

| Cơ chế | Bắt được | Bỏ sót |
|--------|-----------|--------|
| `select='gt(scene,0.30)'` | Cắt cảnh đột ngột, chuyển slide | Slide hiện từ từ, viết thêm công thức |
| Sampling mỗi 5s + so sánh dHash | Thay đổi từ từ | (tốn thêm compute) |

**Đề xuất:** chạy cả hai, gộp kết quả, dedupe bằng dHash (tái dùng code V1).

Ngưỡng khởi đầu: `scene > 0.30`, sampling 5s, dHash threshold 6 (giống V1).

### 5.3. Caption → Scene mapping

PDF của ytb2pdf được tổ chức theo **caption** (mỗi trang = đoạn văn + ảnh).
V2 chụp frame theo **scene**, nên cần map:

```
caption_segment (t=123.5s) → nearest scene frame (t=120.0s hoặc 125.0s)
```

Quy tắc: lấy scene frame có timestamp **≤** caption timestamp và gần nhất.
Nếu không có (caption ở đầu video trước scene đầu tiên), dùng scene đầu tiên.

Sau mapping, tái dùng logic scene compacting của V1 để gộp.

### 5.4. Segment planning: gộp để giảm số lần download

Thay vì gọi `yt-dlp` 100 lần cho 100 scene:

```
Scene timestamps: 12.5, 14.2, 15.8, 95.8, 97.1, 220.4
     ↓ gộp (khoảng cách < 10s)
Segments: [10-18], [93-100], [218-223]
     ↓ 3 lần download
```

Mỗi segment download 1 lần, trích nhiều frame từ đó bằng ffmpeg seek local
(nhanh, không tốn bandwidth thêm).

## 6. Rủi ro & giảm thiểu

| Rủi ro | Mức độ | Giảm thiểu |
|--------|--------|------------|
| `--download-sections` không tiết kiệm bandwidth với một số format | Cao | ✅ Đã validate — tiết kiệm thật. Fallback về V1 nếu segment > 50% video |
| Timestamp lệch do keyframe | Trung bình | `--force-keyframes-at-cuts` + padding 2s mỗi đầu |
| Video không có bản 240p | Thấp | Fallback: dùng bản thấp nhất có sẵn, hoặc về V1 |
| Scene detection bỏ sót slide quan trọng | Trung bình | Kết hợp periodic sampling (mục 5.2) |
| Quá nhiều segment → nhiều lần download | Thấp | Segment planner gộp (mục 5.4); giới hạn 200 segment |
| YouTube throttle mỗi connection | Trung bình | Download segment song song (`--jobs`), retry kiên nhẫn (đã có ở V1) |

## 7. Kế hoạch thực hiện

### Phase 0: Validate (✅ DONE — 2026-10-15)
- [x] Test `--download-sections` tiết kiệm bandwidth thật
- [x] Test trích frame từ segment

### Phase 1: Proxy + Scene Detection (✅ DONE — 2026-10-15)
- [x] Module `v2/proxy.rs`: download 240p
- [x] Module `v2/scene.rs`: ffmpeg scene detection + periodic sampling
- [x] Module `v2/segments.rs`: gộp timestamp thành khoảng
- [x] Module `v2/hi_res.rs`: download song song các segment ở 720p
- [x] Module `v2/mapping.rs`: map caption → scene frame
- [x] 17 unit tests + 1 integration test xanh
- [x] Đã push 7 files lên GitHub main (SHA-verified)

### Phase 2: Segment Planning + High-res Download (✅ DONE — 2026-10-15)
- [x] Download song song các segment (parallel, resumable)
- [x] Tái dùng logic retry/backoff từ V1

### Phase 3: Frame Extraction + Caption Mapping (✅ DONE — 2026-10-15)
- [x] Module `v2/pipeline.rs`: orchestrator 9 bước
- [x] Trích frame từ segment (tái dùng `extract_frame`)
- [x] Map caption → scene frame gần nhất
- [x] Tích hợp vào CLI với flag `--v2`
- [x] Đã push 5 files lên GitHub main (SHA-verified)

### Phase 4: Testing + Docs (🔄 IN PROGRESS — 2026-10-15)
- [x] Test trên video 30 phút (Nobel Kinh tế 2026): V2 chạy end-to-end thành công
- [x] So sánh V1 vs V2 (xem mục 10 bên dưới)
- [ ] Test trên video 3h+ (đo bandwidth tiết kiệm) — CS50 đang chạy V1, chưa test V2
- [ ] Cập nhật AGENTS.md + README với flag `--v2`

## 8. Tiêu chí thành công

V2 được coi là thành công khi:

1. **Bandwidth:** download < 20% so với V1 cho video 1h+ (đo thực tế).
2. **Chất lượng frame:** không phân biệt được V1 vs V2 bằng mắt thường.
3. **PDF output:** số trang ±10% so với V1, cùng nội dung caption.
4. **Thời gian:** tổng thời gian V2 < 70% V1 cho video 3h+.

### Kết quả thực tế (video Nobel 30 phút, 2026-10-15)

| Metric | V1 | V2 | Đánh giá |
|--------|----|----|----------|
| Download | 101MB (full 720p) | 76MB (15MB proxy + 54MB segments) | ⚠️ Tiết kiệm 25% (chưa đạt 20% target) |
| Scene phát hiện | 110 (caption-based) | 94 (visual-based) | ✅ Tương đương |
| PDF | 104MB, 110 trang | 80MB, 85 trang | ⚠️ Ít hơn 23% (vượt ±10%) |
| Chất lượng frame | 720p | 720p | ✅ Không phân biệt được |

**Nhận xét:** Video 30 phút với 94 scene dày đặc thì V2 chỉ tiết kiệm 25%.
V2 thực sự tỏa sáng với video DÀI và ÍT scene (như CS50 12 tiếng) — ước tính tiết kiệm 60-70%.
Cần test thêm trên video 3h+ để xác nhận.

## 10. Bugs phát hiện & fixes (2026-10-15)

### Bug 1: Sampling quá dày → gộp thành 1 segment
- **Triệu chứng:** Sampling mỗi 5s tạo 379 "scene", tất cả gộp thành 1 segment duy nhất (= download full video).
- **Fix:** Sampling thưa ra 30s, chỉ lấp khoảng trống >15s không có scene nào.
- **Kết quả:** 94 scene → 58 segment (đúng thiết kế).

### Bug 2: Segment bị cắt ngắn ở cuối
- **Triệu chứng:** `--download-sections` tạo file có metadata duration 1785s nhưng data thực chỉ ~1700s. Scene ở 1710s không trích được frame → cả pipeline fail.
- **Fix:** Bỏ qua scene không trích được frame (warning) thay vì fail cả pipeline. Áp dụng cho cả V1 (`frames.rs`) và V2 (`v2/pipeline.rs`).

### Bug 3: OOM khi dựng PDF 7,628 trang
- **Triệu chứng:** genpdf giữ tất cả ảnh trong RAM → process bị kill (EXIT:137) ở bước dựng PDF cho video CS50 12 tiếng.
- **Fix:** Chia blocks thành chunk 1,500 trang, build từng chunk riêng, gộp bằng `pdfunite` (poppler-utils).
- **Kết quả:** CS50 PDF 1.9GB, 7,628 trang hoàn thành thành công.

## 11. Bonus: `--text-only` mode (2026-10-15)

Phát sinh từ nhu cầu thực tế: PDF 1.9GB quá lớn để xem trên điện thoại.

- Flag `--text-only`: bỏ qua download video, bỏ qua chụp frame, chỉ giữ caption.
- Kết quả: CS50 text-only PDF **51MB** (thay vì 1.9GB), 14,295 trang — nhẹ hơn 37 lần.
- Phù hợp đọc trên mobile, vẫn có link YouTube ở footer.

## 9. Câu hỏi mở

1. **Có nên áp dụng V2 cho app GUI không?** Hay chỉ CLI/agent? (Đề xuất: chỉ CLI trước)
2. **Ngưỡng scene detection tối ưu cho từng loại video?** (Cần tune thực tế: bài giảng vs vlog vs phim)
3. **Có nên cache proxy.mp4 giữa các lần chạy?** (Tiết kiệm nếu user chạy lại với caption khác)

---

## Phụ lục: Module structure đề xuất

```
crates/ytb2pdf-core/src/
├── v2/
│   ├── mod.rs           # V2 pipeline orchestrator
│   ├── proxy.rs         # 240p download
│   ├── scene.rs         # Scene detection (ffmpeg + sampling)
│   ├── segments.rs      # Segment planning (merge timestamps)
│   ├── hi_res.rs        # High-res segment download
│   └── mapping.rs       # Caption → scene frame mapping
```

Hoặc đơn giản hơn: thêm vào module hiện có (`youtube.rs`, `frames.rs`)
với prefix `v2_`. Quyết định khi implement Phase 1.
