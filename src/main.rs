fn main() {
    let source = std::fs::read("./qoi_test_images/".to_owned() + "dice.qoi").unwrap();
    let (_, my_decoding) = qoi::decode(&source, None).unwrap();
    let mut file = std::fs::File::create("./target/bla.png").unwrap();
    image::write_buffer_with_format(
        &mut file,
        &my_decoding.data,
        my_decoding.header.width,
        my_decoding.header.height,
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )
    .unwrap();
}
