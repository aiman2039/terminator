use super::*;

#[test]
fn only_binary_raster_images_switch_to_the_image_diff() {
    let binary = "Binary file · 4 B → 3 B. Binary or non-UTF-8 files cannot be reviewed natively.";
    assert!(is_binary_image(Path::new("a/logo.PNG"), binary));
    assert!(is_binary_image(
        Path::new("photo.jpeg"),
        "Diff file exceeds 1 MiB"
    ));
    assert!(!is_binary_image(Path::new("icon.svg"), binary));
    assert!(!is_binary_image(Path::new("data.bin"), binary));
    assert!(!is_binary_image(
        Path::new("logo.png"),
        "This file has no staged changes; refresh Git status"
    ));
}

#[test]
fn decoding_reports_size_dimensions_and_absent_sides() {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    let size = bytes.len();
    let decoded = decode(Some(bytes));
    assert_eq!(decoded.bytes, size);
    assert_eq!(decoded.dims, Some([3, 2]));
    assert_eq!(
        decoded.image.unwrap().unwrap().size,
        [3, 2],
        "decoded pixels"
    );
    let absent = decode(None);
    assert!(absent.image.is_none());
    assert!(
        decode(Some(b"not an image".to_vec()))
            .image
            .unwrap()
            .is_err()
    );
}
