use crate::background::gray;

pub struct Features {
    pub idf: f64,
    pub skewness: f64,
    pub kurtosis: f64,
    pub hist_peaks: usize,
    pub rel_peaks: f64,
    pub nns_argmax: usize,
    pub transparency_levels: usize,
    pub nns_std_err: f64,
    pub otsu_partition_score: f64,
}

const NNS_BINS: usize = 20;
const NNS_RANGE_MAX: f64 = 4.0;
const AVERAGING_LEN: usize = 10;

pub fn image_features(rgba: &[u8], width: usize, height: usize) -> Features {
    let n_pixels = width * height;
    let mut gray_hist = [0u64; 256];
    let mut alpha_levels = 0u16;
    let mut peaks: Vec<f64> = Vec::new();
    let mut peak_tracker = PeakTracker::new(n_pixels);

    let mut rows_identical = true;
    let mut first_row = Vec::with_capacity(width);

    for (i, px) in rgba.as_chunks::<4>().0.iter().enumerate() {
        let g = gray(px[0], px[1], px[2]);
        if i < width {
            first_row.push(g);
        } else if g != first_row[i % width] {
            rows_identical = false;
        }
        gray_hist[g as usize] += 1;
        alpha_levels |= 1 << (px[3] >> 4);
        peak_tracker.push(
            (px[0] >> 4) + (px[1] >> 4) + (px[2] >> 4) + (px[3] >> 4),
            &mut peaks,
        );
    }
    peak_tracker.finish(&mut peaks);

    let corner = |x: usize, y: usize| {
        let i = (y * width + x) * 4;
        gray(rgba[i], rgba[i + 1], rgba[i + 2]) as usize
    };
    let corners = [
        corner(0, 0),
        corner(width - 1, 0),
        corner(0, height - 1),
        corner(width - 1, height - 1),
    ];

    let hist_mean = n_pixels as f64 / 256.0;
    let intensive = gray_hist.iter().filter(|&&h| h as f64 > hist_mean).count();

    let (nns_argmax, nns_std_err) = nns_features(&peaks);
    let (skewness, kurtosis) = skew_kurtosis(&gray_hist, n_pixels, rows_identical);

    Features {
        // numpy 2 yields inf here rather than raising when nothing is above the mean
        idf: (256.0 / intensive as f64).ln(),
        skewness,
        kurtosis,
        hist_peaks: significant_hist_peaks(&gray_hist, hist_mean * 20.0),
        rel_peaks: peaks.len() as f64 / ((width + height) as f64 / 2.0),
        nns_argmax,
        transparency_levels: alpha_levels.count_ones() as usize,
        nns_std_err,
        otsu_partition_score: otsu_partition_score(&gray_hist, n_pixels, corners),
    }
}

fn significant_hist_peaks(hist: &[u64; 256], min_height: f64) -> usize {
    (0..256)
        .filter(|&i| {
            let rising = i == 0 || hist[i] > hist[i - 1];
            let falling = i == 255 || hist[i + 1] < hist[i];
            rising && falling && hist[i] as f64 >= min_height
        })
        .count()
}

/// The reference tests `image_array == image_array[0]`, which compares every
/// row with the first row, so any image with identical rows counts as constant.
fn skew_kurtosis(hist: &[u64; 256], n: usize, rows_identical: bool) -> (f64, f64) {
    if rows_identical {
        return (0.0, -3.0);
    }
    let total = n as f64;
    let sum: u64 = hist.iter().enumerate().map(|(v, &h)| v as u64 * h).sum();
    let mean = sum as f64 / total;
    let (mut m2, mut m3, mut m4) = (0.0, 0.0, 0.0);
    for (v, &h) in hist.iter().enumerate() {
        let c = v as f64 - mean;
        let c2 = c * c;
        m2 += h as f64 * c2;
        m3 += h as f64 * c2 * c;
        m4 += h as f64 * c2 * c2;
    }
    let variance = m2 / total;
    (
        (m3 / total / variance.powf(1.5)).abs(),
        m4 / total / (variance * variance) - 3.0,
    )
}

fn otsu_partition_score(hist: &[u64; 256], n: usize, corners: [usize; 4]) -> f64 {
    let min = hist.iter().position(|&h| h > 0).unwrap();
    let max = hist.iter().rposition(|&h| h > 0).unwrap();
    if min == max {
        return 0.0;
    }
    let counts: Vec<f64> = hist[min..=max].iter().map(|&h| h as f64).collect();
    let len = counts.len();
    let centers: Vec<f64> = (min..=max).map(|v| v as f64).collect();

    let mut weight1 = Vec::with_capacity(len);
    let mut mean1 = Vec::with_capacity(len);
    let (mut w, mut s) = (0.0, 0.0);
    for i in 0..len {
        w += counts[i];
        s += counts[i] * centers[i];
        weight1.push(w);
        mean1.push(s / w);
    }
    let mut weight2 = vec![0.0; len];
    let mut mean2 = vec![0.0; len];
    let (mut w, mut s) = (0.0, 0.0);
    for i in (0..len).rev() {
        w += counts[i];
        s += counts[i] * centers[i];
        weight2[i] = w;
        mean2[i] = s / w;
    }

    let mut best = 0;
    let mut best_variance = f64::NEG_INFINITY;
    for i in 0..len - 1 {
        let d = mean1[i] - mean2[i + 1];
        let variance = weight1[i] * weight2[i + 1] * (d * d);
        if variance > best_variance {
            best_variance = variance;
            best = i;
        }
    }
    let threshold = centers[best];

    let white = hist
        .iter()
        .enumerate()
        .filter(|&(v, _)| v as f64 > threshold)
        .map(|(_, &h)| h)
        .sum::<u64>();
    let white_ratio = white as f64 / n as f64;
    let white_corners = corners.iter().filter(|&&c| c as f64 > threshold).count();
    let signal_ratio = if white_corners <= 2 {
        white_ratio
    } else {
        1.0 - white_ratio
    };
    let target = 0.14;
    signal_ratio.min(target) / signal_ratio.max(target)
}

/// Streaming port of intensity_statistics.detect_peaks: plateau-aware local
/// maxima, positioned at the plateau centre.
struct PeakTracker {
    n: usize,
    index: usize,
    prev: u8,
    run_start: usize,
    entering_up: bool,
    changed: bool,
}

impl PeakTracker {
    fn new(n: usize) -> Self {
        Self {
            n,
            index: 0,
            prev: 0,
            run_start: 0,
            entering_up: true,
            changed: false,
        }
    }

    fn push(&mut self, value: u8, peaks: &mut Vec<f64>) {
        if self.index > 0 && value != self.prev {
            self.close_run(self.index, value < self.prev, peaks);
            self.run_start = self.index;
            self.entering_up = value > self.prev;
            self.changed = true;
        }
        self.prev = value;
        self.index += 1;
    }

    fn finish(&mut self, peaks: &mut Vec<f64>) {
        if self.n >= 3 && self.changed {
            self.close_run(self.n, true, peaks);
        }
    }

    fn close_run(&self, end_exclusive: usize, leaving_down: bool, peaks: &mut Vec<f64>) {
        if self.n >= 3 && self.entering_up && leaving_down {
            peaks.push((self.run_start + end_exclusive - 1) as f64 / 2.0);
        }
    }
}

fn nns_features(peaks: &[f64]) -> (usize, f64) {
    let spacings: Vec<f64> = peaks.windows(2).map(|w| w[1] - w[0]).collect();
    let nns = unfolded_nns(&spacings);

    let edges: Vec<f64> = (0..=NNS_BINS)
        .map(|i| {
            if i == NNS_BINS {
                NNS_RANGE_MAX
            } else {
                i as f64 * (NNS_RANGE_MAX / NNS_BINS as f64)
            }
        })
        .collect();
    let mut counts = [0u64; NNS_BINS];
    for &x in &nns {
        if !(0.0..=NNS_RANGE_MAX).contains(&x) {
            continue;
        }
        let mut i = (x * (NNS_BINS as f64 / NNS_RANGE_MAX)) as usize;
        if i == NNS_BINS {
            i -= 1;
        }
        if x < edges[i] {
            i -= 1;
        }
        if x >= edges[i + 1] && i != NNS_BINS - 1 {
            i += 1;
        }
        counts[i] += 1;
    }

    let total: u64 = counts.iter().sum();
    if total == 0 {
        return (0, f64::NAN);
    }
    let density: Vec<f64> = (0..NNS_BINS)
        .map(|i| counts[i] as f64 / (edges[i + 1] - edges[i]) / total as f64)
        .collect();
    let mut argmax = 0;
    for (i, &d) in density.iter().enumerate() {
        if d > density[argmax] {
            argmax = i;
        }
    }
    let squared: Vec<f64> = (0..NNS_BINS)
        .map(|i| {
            let centre = (edges[i + 1] + edges[i]) / 2.0;
            let d = density[i] - (-centre).exp();
            d * d
        })
        .collect();
    (argmax, (numpy_sum(&squared) / NNS_BINS as f64).sqrt())
}

fn unfolded_nns(spacings: &[f64]) -> Vec<f64> {
    if spacings.is_empty() {
        return vec![1.0];
    }
    let window = 2 * AVERAGING_LEN + 1;
    let size = spacings.len();
    if size < window {
        return spacings.iter().map(|_| 1.0).collect();
    }
    let mut padded = Vec::with_capacity(size + window);
    padded.push(0.0);
    padded.extend_from_slice(&spacings[..AVERAGING_LEN]);
    padded.extend_from_slice(spacings);
    padded.extend_from_slice(&spacings[size - AVERAGING_LEN..]);
    let mut cumulative = Vec::with_capacity(padded.len());
    let mut total = 0.0;
    for p in &padded {
        total += p;
        cumulative.push(total);
    }
    (0..size)
        .map(|i| spacings[i] / ((cumulative[window + i] - cumulative[i]) / window as f64))
        .collect()
}

/// numpy's pairwise summation for short contiguous arrays (8 interleaved
/// accumulators), so the mean matches np.mean bit for bit.
fn numpy_sum(a: &[f64]) -> f64 {
    if a.len() < 8 {
        return a.iter().fold(0.0, |acc, x| acc + x);
    }
    let mut r: [f64; 8] = a[..8].try_into().unwrap();
    let blocks_end = a.len() - a.len() % 8;
    let mut i = 8;
    while i < blocks_end {
        for j in 0..8 {
            r[j] += a[i + j];
        }
        i += 8;
    }
    let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
    for x in &a[blocks_end..] {
        res += x;
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, px: [u8; 4]) -> Vec<u8> {
        px.iter().copied().cycle().take(w * h * 4).collect()
    }

    #[test]
    fn constant_image_has_no_shape_features() {
        let f = image_features(&solid(8, 8, [10, 20, 30, 255]), 8, 8);
        assert_eq!(
            (f.skewness, f.kurtosis, f.otsu_partition_score),
            (0.0, -3.0, 0.0)
        );
        assert_eq!((f.rel_peaks, f.transparency_levels), (0.0, 1));
    }

    #[test]
    fn plateau_peak_sits_at_plateau_centre() {
        let mut tracker = PeakTracker::new(7);
        let mut peaks = Vec::new();
        for v in [0u8, 1, 3, 3, 3, 1, 0] {
            tracker.push(v, &mut peaks);
        }
        tracker.finish(&mut peaks);
        assert_eq!(peaks, vec![3.0]);
    }

    #[test]
    fn uniform_gray_levels_have_infinite_idf() {
        let rgba: Vec<u8> = (0..=255u8).flat_map(|v| [v, v, v, 255]).collect();
        assert!(image_features(&rgba, 16, 16).idf.is_infinite());
    }

    #[test]
    fn identical_rows_count_as_constant() {
        let rgba: Vec<u8> = (0..8u8)
            .flat_map(|x| [x * 30, 0, 0, 255])
            .cycle()
            .take(8 * 4 * 5)
            .collect();
        let f = image_features(&rgba, 8, 5);
        assert_eq!((f.skewness, f.kurtosis), (0.0, -3.0));
    }

    #[test]
    fn numpy_sum_matches_sequential_for_short_input() {
        assert_eq!(numpy_sum(&[1.0, 2.0, 3.0]), 6.0);
    }
}
