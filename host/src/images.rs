//! Images an app hands over — `image-load` and `ui-image` in the WIT.
//!
//! Decoded once, on the host, and kept as RGBA behind an `Arc`: the frame's UI
//! commands are cloned every frame, and a picture inside them must cost a
//! pointer to clone, not its pixels. A guest is untrusted, and a few bytes of
//! PNG can claim to be any size, so the size is checked before anything is
//! allocated for it.

use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// The largest image, on either side: 4096 pixels.
pub const MAX_IMAGE_SIDE: u32 = 4096;
/// The most pixels one app's images may hold together: 8 megapixels, 32 MiB of
/// RGBA.
pub const MAX_APP_PIXELS: u64 = 8 << 20;

/// A decoded image, ready to draw.
#[derive(Clone)]
pub struct Picture {
    /// Unique across every app the shell runs, so a renderer can cache a
    /// texture by it without two apps' image 0 colliding.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) RGBA, `width * height * 4` bytes.
    pub rgba: Arc<[u8]>,
}

impl std::fmt::Debug for Picture {
    /// The size, not a few million bytes of pixels.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture#{} {}x{}", self.id, self.width, self.height)
    }
}

/// One app's images, numbered in the order it loaded them.
#[derive(Default)]
pub struct Images {
    list: Vec<Picture>,
    pixels: u64,
}

impl Images {
    /// Decode `bytes` and keep the result; the number is what `ui-image`
    /// draws it by.
    pub fn load(&mut self, bytes: &[u8]) -> Result<u32, String> {
        let reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| format!("not an image: {e}"))?;
        // The header, before any pixel is decoded or memory set aside.
        let (width, height) = reader
            .into_dimensions()
            .map_err(|e| format!("not an image this shell reads (PNG or JPEG): {e}"))?;
        if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
            return Err(format!(
                "image is {width} x {height}; the most is {MAX_IMAGE_SIDE} on a side"
            ));
        }
        let pixels = u64::from(width) * u64::from(height);
        if self.pixels + pixels > MAX_APP_PIXELS {
            return Err(format!(
                "an app's images may hold {} megapixels in all",
                MAX_APP_PIXELS >> 20
            ));
        }

        // Decoded with the same limits, in case the header and the data do
        // not agree about what is coming.
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| format!("not an image: {e}"))?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_SIDE);
        limits.max_image_height = Some(MAX_IMAGE_SIDE);
        limits.max_alloc = Some(pixels * 4 + (1 << 20));
        reader.limits(limits);
        let rgba = reader
            .decode()
            .map_err(|e| format!("could not decode the image: {e}"))?
            .into_rgba8();

        static NEXT: AtomicU64 = AtomicU64::new(0);
        self.pixels += pixels;
        self.list.push(Picture {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            rgba: rgba.into_raw().into(),
        });
        Ok(self.list.len() as u32 - 1)
    }

    pub fn get(&self, handle: u32) -> Option<&Picture> {
        self.list.get(handle as usize)
    }
}
