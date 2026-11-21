//! A transparent 8-bit RGBA grid for authoring small map symbols.
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_SIDE: u16 = 128;
pub type Rgba = [u8; 4];

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PixelArt {
    width: u16,
    height: u16,
    pixels: Vec<Rgba>,
}

impl PixelArt {
    pub fn new(width: u16, height: u16) -> Result<Self, String> {
        let len = pixel_count(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![[0; 4]; len],
        })
    }

    pub fn from_pixels(width: u16, height: u16, mut pixels: Vec<Rgba>) -> Result<Self, String> {
        if pixel_count(width, height)? != pixels.len() {
            return Err("像素图数据量与画布尺寸不一致。".into());
        }
        for p in &mut pixels {
            if p[3] == 0 {
                *p = [0; 4];
            }
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn size(&self) -> [u16; 2] {
        [self.width, self.height]
    }

    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }

    pub fn has_ink(&self) -> bool {
        self.pixels.iter().any(|p| p[3] > 0)
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.width && y < self.height)
            .then_some(usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    pub fn pixel(&self, at: [u16; 2]) -> Option<Rgba> {
        self.index(at[0], at[1]).map(|i| self.pixels[i])
    }

    /// Resize from the top left, keeping the overlap with the old grid.
    pub fn resized(&self, width: u16, height: u16) -> Result<Self, String> {
        let mut result = Self::new(width, height)?;
        let overlap = usize::from(width.min(self.width));
        for y in 0..usize::from(height.min(self.height)) {
            let lo = usize::from(self.width) * y;
            let dst = usize::from(width) * y;
            result.pixels[dst..dst + overlap].copy_from_slice(&self.pixels[lo..lo + overlap]);
        }
        Ok(result)
    }

    /// Paint every integer cell on the path, including its endpoints.
    /// Each brush stamp replaces the cell's RGBA value, making a repeat
    /// sample identical to a single sample. A transparent stamp erases.
    pub fn stroke(&mut self, from: [u16; 2], to: [u16; 2], size: u16, color: Rgba) -> bool {
        if self.pixel(from).is_none() || self.pixel(to).is_none() {
            return false;
        }
        let [mut x, mut y] = from.map(i32::from);
        let [tx, ty] = to.map(i32::from);
        let (dx, dy) = ((tx - x).abs(), -(ty - y).abs());
        let (sx, sy) = ((tx - x).signum(), (ty - y).signum());
        let mut err = dx + dy;
        let size = i32::from(size.clamp(1, MAX_SIDE));
        let color = if color[3] == 0 { [0; 4] } else { color };
        let mut changed = false;
        loop {
            let left = x - (size - 1) / 2;
            let top = y - (size - 1) / 2;
            for row in top.max(0)..(top + size).min(i32::from(self.height)) {
                let base = row as usize * usize::from(self.width);
                for col in left.max(0)..(left + size).min(i32::from(self.width)) {
                    let px = &mut self.pixels[base + col as usize];
                    changed |= *px != color;
                    *px = color;
                }
            }
            if x == tx && y == ty {
                return changed;
            }
            let doubled = 2 * err;
            if doubled >= dy {
                err += dy;
                x += sx;
            }
            if doubled <= dx {
                y += sy;
                err += dx;
            }
        }
    }

    /// Fits the longest side in the given square while retaining aspect.
    pub fn drawn_size(&self, extent: f32) -> [f32; 2] {
        let scale = extent / f32::from(self.width.max(self.height));
        [
            f32::from(self.width) * scale,
            f32::from(self.height) * scale,
        ]
    }

    /// Non-transparent runs for exporters that draw vector rectangles.
    pub fn runs(&self) -> impl Iterator<Item = ([u16; 3], Rgba)> + '_ {
        self.pixels
            .chunks_exact(usize::from(self.width))
            .enumerate()
            .flat_map(|(y, row)| {
                let mut x = 0;
                std::iter::from_fn(move || loop {
                    let start = x;
                    let color = *row.get(x)?;
                    x += row[x..].iter().take_while(|&&p| p == color).count();
                    if color[3] != 0 {
                        return Some(([start as u16, x as u16, y as u16], color));
                    }
                })
            })
    }
}

fn pixel_count(width: u16, height: u16) -> Result<usize, String> {
    if (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height) {
        Ok(usize::from(width) * usize::from(height))
    } else {
        Err(format!("像素图边长应在 1–{MAX_SIDE} 之间。"))
    }
}

impl<'de> Deserialize<'de> for PixelArt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Stored {
            width: u16,
            height: u16,
            pixels: Vec<Rgba>,
        }

        let Stored {
            width,
            height,
            pixels,
        } = Stored::deserialize(deserializer)?;
        Self::from_pixels(width, height, pixels).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_strokes_and_erasing_use_exact_cells() {
        let mut art = PixelArt::new(16, 8).unwrap();
        let red = [230, 20, 32, 255];
        assert!(art.stroke([0, 0], [15, 7], 1, red));
        assert_eq!(art.pixels().iter().filter(|&&p| p == red).count(), 16);
        assert_eq!(art.pixel([15, 7]), Some(red));
        assert!(art.stroke([15, 7], [15, 7], 3, [255, 255, 255, 0]));
        assert_eq!(art.pixel([14, 6]), Some([0; 4]));
        assert_eq!(art.resized(4, 4).unwrap().pixel([0, 0]), Some(red));
    }
}
