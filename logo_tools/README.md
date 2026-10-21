# logo_tools

Numeric kernels for logo scoring, called from Python:

- `image_features(rgba, width, height)` returns `(IMAGE_IDF, SKEWNESS, KURTOSIS, HIST_PEAKS, REL_PEAKS, NNS_ARGMAX, TRANSPARENCY_LEVELS, NNS_STD_ERR, OTSU_PARTITION_SCORE)` for an RGBA image.
- `needs_gray_background(rgba, width, height, light_image_threshold)` decides whether a partly transparent logo needs a gray background.

Both reproduce the numpy/Pillow implementation they replace, including its quirks (rows identical to row 0 count as a constant image; `REL_PEAKS` divides by `(width + height) / 2`; `IMAGE_IDF` is `inf` when no gray level is above the mean), and release the GIL.

```
cd logo_tools
cargo test
maturin develop --release
```
