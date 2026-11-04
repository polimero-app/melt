use super::{MAX_PREVIEW_BYTES, MAX_PREVIEW_DIMENSION, MAX_PREVIEW_PIXELS, PreviewError};
use image::ImageReader;
use std::io::Cursor;

/// Decode before accepting an embedded image. All consumers and caches receive
/// bounded PNG data, regardless of the source encoding.
pub(super) fn normalize(bytes: &[u8]) -> Result<Vec<u8>, PreviewError> {
    if bytes.len() > MAX_PREVIEW_BYTES {
        return Err(PreviewError::TooLarge);
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| PreviewError::InvalidModel)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_PREVIEW_DIMENSION);
    limits.max_image_height = Some(MAX_PREVIEW_DIMENSION);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| PreviewError::InvalidModel)?;
    if u64::from(decoded.width()) * u64::from(decoded.height()) > MAX_PREVIEW_PIXELS {
        return Err(PreviewError::TooLarge);
    }
    // Preserve the original PNG, including its colour/alpha information.
    if super::validate_png(bytes) {
        return Ok(bytes.to_vec());
    }
    let mut output = Cursor::new(Vec::new());
    decoded
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|_| PreviewError::Png)?;
    let output = output.into_inner();
    if output.len() > MAX_PREVIEW_BYTES {
        return Err(PreviewError::TooLarge);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_jpeg_and_rejects_truncated_images() {
        let mut jpeg = Cursor::new(Vec::new());
        image::RgbImage::from_pixel(7, 3, image::Rgb([20, 80, 120]))
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let png = normalize(jpeg.get_ref()).unwrap();
        assert!(super::super::validate_png(&png));
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (7, 3));
        assert!(normalize(&png[..24]).is_err());
    }
}
