#![allow(clippy::needless_for_each)]

const TEST_IMAGES_DIR: &str = "./qoi_test_images";

fn main() {
    let qoi_files = [
        "dice",
        "kodim10",
        "kodim23",
        "qoi_logo",
        "testcard",
        "testcard_rgba",
        "wikipedia_008",
    ];

    qoi_files.iter().for_each(|name| {
        let qoi_data = std::fs::read(format!("{TEST_IMAGES_DIR}/{name}.qoi"))
            .expect("test qoi files should exist in the repo");
        let (_, qoi_decoded) = qoi::decode(&qoi_data, None).expect("qoi files should be valid");

        let png = image::open(format!("{TEST_IMAGES_DIR}/{name}.png"))
            .expect("png files should exist in the repo");

        let qoi_bytes = qoi_decoded.data.as_ref();
        let png_bytes = match qoi_decoded.header.channel {
            qoi::Channels::Rgba => png.to_rgba8().into_raw(),
            qoi::Channels::Rgb => png.to_rgb8().into_raw(),
        };

        assert_eq!(qoi_bytes, png_bytes, "{name}: pixel data mismatch");
    });
}
