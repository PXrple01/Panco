//! Bildschirmfotos für Claude: verkleinern (lange Seite höchstens 1568 px) und als PNG kodieren.
//! Alles nur im Speicher – nichts landet auf der Festplatte.

use crate::platform::Image;

/// Längste Seite, die Claude ohne eigenes Verkleinern verarbeitet.
pub const MAX_SIDE: u32 = 1568;

/// Auf höchstens `max` Pixel (lange Seite) verkleinern – Mittelwert über Pixelblöcke.
pub fn downscale(img: &Image, max: u32) -> Image {
    let long = img.width.max(img.height);
    if long <= max || img.width == 0 || img.height == 0 {
        return img.clone();
    }
    let (w, h) = (
        ((img.width as u64 * max as u64) / long as u64).max(1) as u32,
        ((img.height as u64 * max as u64) / long as u64).max(1) as u32,
    );
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let (y0, y1) = (y * img.height / h, ((y + 1) * img.height / h).max(y * img.height / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * img.width / w, ((x + 1) * img.width / w).max(x * img.width / w + 1));
            let mut sum = [0u32; 4];
            let mut n = 0;
            for sy in y0..y1.min(img.height) {
                for sx in x0..x1.min(img.width) {
                    let i = ((sy * img.width + sx) * 4) as usize;
                    for c in 0..4 {
                        sum[c] += img.rgba[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let o = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                out[o + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }
    Image { width: w, height: h, rgba: out }
}

pub fn to_png(img: &Image) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&img.rgba).map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_and_encode() {
        let img = Image { width: 3200, height: 1800, rgba: vec![128; 3200 * 1800 * 4] };
        let small = downscale(&img, MAX_SIDE);
        assert_eq!((small.width, small.height), (1568, 882));
        assert!(small.rgba.iter().all(|&b| b == 128));
        let png = to_png(&small).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let tiny = Image { width: 2, height: 1, rgba: vec![0, 0, 0, 255, 255, 255, 255, 255] };
        assert_eq!(downscale(&tiny, MAX_SIDE), tiny);
    }
}
