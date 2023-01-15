use anyhow::Result;
use clap::Parser;
use fltk::{
    app::App,
    enums::ColorDepth::Rgba8,
    frame::Frame,
    image::{PngImage, RgbImage},
    prelude::*,
    valuator::HorNiceSlider,
    window::Window,
};
use rgb::AsPixels;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    #[arg()]
    path: PathBuf,
}

fn main() -> Result<()> {
    // Initialize app
    let args = Args::parse();
    let app = App::default();
    let mut window = Window::default().with_size(640, 360 + 24 + 4 * 2);
    let mut frame = Frame::default().size_of(&window);
    let mut slider = HorNiceSlider::default()
        .with_pos(4, 360 + 4)
        .with_size(640 - 4 * 2, 24);
    slider.set_minimum(0.0);
    slider.set_maximum(100.0);
    slider.set_step(1.0, 1);
    slider.set_value(50.0);
    window.end();
    window.make_resizable(true);
    window.show();

    // Load source
    let source = PngImage::load(args.path)?;
    if source.depth() != Rgba8 {
        unimplemented!("color mode {:?}", source.depth())
    }
    let source_rgba = source.to_rgb_data();

    // Initialize quantizer
    let mut quantizer = imagequant::new();

    // Register event handler
    slider.set_callback(move |s| {
        let quality = (s.value().round() as i64).try_into().unwrap(); // Pending https://github.com/rust-lang/rust/issues/67057

        // Quantize
        quantizer.set_speed(10).unwrap();
        quantizer.set_quality(0, quality).unwrap();
        let mut source_pixels = quantizer
            .new_image_borrowed(
                source_rgba.as_pixels(),
                source.width() as usize,
                source.height() as usize,
                0.0,
            )
            .unwrap();
        let mut quantization = quantizer.quantize(&mut source_pixels).unwrap();
        quantization.set_dithering_level(0.0).unwrap();
        let (palette, quantized_indexed) = quantization.remapped(&mut source_pixels).unwrap();

        // Display
        let quantized_rgba = quantized_indexed
            .iter()
            .flat_map(|&i| palette[i as usize].iter())
            .collect::<Vec<_>>();
        let image = RgbImage::new(&quantized_rgba, source.w(), source.h(), Rgba8).unwrap();
        frame.set_image(Some(image));
        frame.redraw();
    });

    // Run
    slider.do_callback();
    app.run()?;
    Ok(())
}
