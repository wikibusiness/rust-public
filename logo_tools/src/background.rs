const TRANSPARENT_PX_THRESHOLD: u8 = 80;
const LIGHT_PX_INTENSITY: u32 = 240;

pub fn gray(r: u8, g: u8, b: u8) -> u32 {
    (r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16
}

pub fn needs_gray_background(rgba: &[u8], light_image_threshold: f64) -> bool {
    let mut visible = 0u64;
    let mut light = 0u64;
    let mut all_opaque = true;
    for px in rgba.as_chunks::<4>().0 {
        if px[3] < TRANSPARENT_PX_THRESHOLD {
            all_opaque = false;
            continue;
        }
        visible += 1;
        if gray(px[0], px[1], px[2]) > LIGHT_PX_INTENSITY {
            light += 1;
        }
    }
    if all_opaque {
        return false;
    }
    light as f64 / (visible as f64 + 1e-20) >= light_image_threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_images_never_get_a_background() {
        assert!(!needs_gray_background(&[255u8; 16], 0.3));
    }

    #[test]
    fn light_visible_pixels_on_transparency_need_gray() {
        let rgba = [[255u8, 255, 255, 255], [0, 0, 0, 0]].concat();
        assert!(needs_gray_background(&rgba, 0.3));
    }

    #[test]
    fn fully_transparent_image_has_no_light_ratio() {
        assert!(!needs_gray_background(&[0, 0, 0, 0], 0.3));
    }
}
