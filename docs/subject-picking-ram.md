# Subject picking: RAM use and how to reduce it

Measured on CachyOS with 22 CPU cores, onnxruntime 1.29 (CPU), on a drawing of 1600×1000 and on a 4000×3000 copy of it.

## Where the memory goes

| Stage | RAM | How long |
|---|---|---|
| quickpeek with a picture open | ~280 MB | always |
| SAM 2.1 models loaded (encoder + decoder) | +360 MB | while picking |
| SAM 2.1 reading the picture | +0.9 GB peak | ~2–9 s |
| Edge model (ToonOut) running | +4.1 GB peak | ~9 s |
| Edge model (BiRefNet lite) running | +6.3 GB peak | ~7 s |
| Each picked part (full-size scores) | +10 MB (1600×1000), +49 MB (4000×3000) | while picking, also kept by Ctrl+Z history |
| Mask and overlay images | +27 MB (1600×1000), +195 MB (4000×3000) | while picking |
| After closing the preview | back to ~300 MB | — |

The peak comes from the edge model. Its working memory at its fixed 1024×1024 input is about 2.4 GB, on top of its weights. The picture size does not change that peak. A bigger picture only grows the per-part and overlay rows.

onnxruntime settings tested on ToonOut:

| Setting | Peak | Time |
|---|---|---|
| default | 4.2 GB | 8.4 s |
| memory pool off (current) | 4.1 GB | 8.8 s |
| 4 threads | 4.1 GB | 11.9 s |
| 1 thread | 4.1 GB | 34.7 s |
| memory pattern off | 4.9 GB | 9.1 s |
| weight prepacking off | 3.7 GB | 9.4 s |

## Options, biggest saving first

### 1. Make edge sharpening opt-in

Run the edge model only when the user clicks a "Sharpen edges" button, instead of every time subject mode starts.

- Saves: the whole 4–6 GB peak for every pick that doesn't need it.
- Costs: one more click. Rough outlines until clicked.

### 2. Use a lighter edge model for drawings

Replace ToonOut (BiRefNet Swin-L, 974 MB) with a smaller anime segmentation model, for example ISNet anime segmentation (skytnt, about 170 MB).

- Saves: likely most of the peak. Not measured: needs the download and a test.
- Costs: lower edge quality is possible. Must be compared on real drawings first.

### 3. Turn off weight prepacking for the edge model

Set `session.disable_prepacking = 1` in `session()` for the edge model only.

- Saves: 400 MB of the peak (measured).
- Costs: about 1 s more per picture.

### 4. Give memory back after each heavy step

Call `malloc_trim(0)` after the picture is read and after the edge model finishes, not only when the window closes. glibc keeps freed blocks otherwise: in a test, trimming after closing went from 2.1 GB to 0.7 GB.

- Saves: about 1 GB of the steady use while picking.
- Costs: a few milliseconds per call.

### 5. Unload the SAM 2.1 encoder after reading the picture

Clicks only use the decoder (16 MB). The encoder (277 MB file) is only needed once per picture.

- Saves: about 300 MB while picking.
- Costs: 1–2 s more on the next picture, to load it again.

### 6. Keep per-part scores at the model's own size

Store each part's 256×256 scores and enlarge them only when merging, instead of keeping a full-size copy per part and per Ctrl+Z step.

- Saves: 10–50 MB per part and per undo step, growing with clicks.
- Costs: about 0.1 s more per click on big pictures.

### 7. Smaller overlay images

Draw the dimming and outline overlays as 8-bit alpha images (`QImage.Format_Alpha8`) instead of 32-bit color.

- Saves: about 75% of the overlay row: ~150 MB at 4000×3000.
- Costs: none expected.

## Not useful

- Fewer threads: same peak, much slower.
- Turning off the memory pattern: higher peak.
- Half-precision (fp16) models: onnxruntime on CPU runs them more than 15× slower.
- Smaller edge-model input: the ToonOut file has a fixed 1024×1024 input.

## Recommended order

1. Option 4 and option 3: small code changes, measured savings, no visible change for the user.
2. Option 1: removes the peak entirely when sharp edges aren't needed.
3. Option 2: only after comparing edge quality on real drawings.
