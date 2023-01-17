use anyhow::Result;
use clap::Parser;
use fltk::{
    app::{self, App},
    enums::ColorDepth::Rgba8,
    frame::Frame,
    image::{PngImage, RgbImage},
    prelude::*,
    valuator::HorNiceSlider,
    window::Window,
};
use rgb::AsPixels;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, RwLock};
use std::thread;

#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    #[arg()]
    path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
struct Params {
    pub dithering: u8,
    pub quality: u8,
    pub speed: u8,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            dithering: 0,
            quality: 25,
            speed: 1,
        }
    }
}

// Pending https://github.com/rust-lang/rust/issues/67057
fn u8_from_f64(n: f64) -> u8 {
    (n.round() as i64).try_into().unwrap()
}

fn main() -> Result<()> {
    let args = Args::parse();
    let params = Arc::new(RwLock::new(Params::default()));
    let (tx, rx) = mpsc::channel();

    // Initialize GUI
    let app = App::default();
    let mut window = Window::default().with_size(640, 360 + 24 + 4 * 2);
    window.make_resizable(true);
    let mut frame = Frame::default().size_of(&window);
    macro_rules! slider {
        ($param:ident, $min:expr, $max:expr, $x:expr, $y:expr, $w:expr, $h:expr) => {{
            let (tx, params) = (tx.clone(), params.clone());
            let mut slider = HorNiceSlider::default().with_pos($x, $y).with_size($w, $h);
            slider.set_minimum($min.into());
            slider.set_maximum($max.into());
            slider.set_step(1.0, 1);
            slider.set_value(params.read().unwrap().$param.into());
            slider.set_callback(move |s| {
                params.write().unwrap().$param = u8_from_f64(s.value());
                tx.send(()).unwrap();
            });
        }};
    }
    slider!(speed, 1, 10, 4, 364, 640 / 4 - 4 - 2, 24);
    slider!(quality, 0, 100, 640 / 4 + 2, 364, 640 / 2 - 4 - 2, 24);
    slider!(dithering, 0, 10, 640 / 4 * 3 + 2, 364, 640 / 4 - 4 - 2, 24);
    window.end();
    window.show();

    // Load source
    let source = PngImage::load(args.path)?;
    if source.depth() != Rgba8 {
        unimplemented!("color mode {:?}", source.depth())
    }
    let source_rgba = source.to_rgb_data();

    // Initialize quantizer
    let mut quantizer = imagequant::new();

    // Initialize worker
    thread::spawn(move || -> Result<()> {
        let mut displayed = None;

        loop {
            rx.recv()?;
            let working = params.read().unwrap().clone();
            match &displayed {
                Some(d) if d == &working => continue,
                _ => {}
            }
            macro_rules! abort_if_untargeted {
                () => {
                    if *params.read().unwrap() != working {
                        continue;
                    }
                };
            }

            // Quantize
            quantizer.set_quality(0, working.quality)?;
            quantizer.set_speed(working.speed.into())?;
            let mut source_pixels = quantizer.new_image_borrowed(
                source_rgba.as_pixels(),
                source.width() as usize,
                source.height() as usize,
                0.0,
            )?;
            abort_if_untargeted!();
            let mut quantization = quantizer.quantize(&mut source_pixels)?;
            abort_if_untargeted!();
            quantization.set_dithering_level(f32::from(working.dithering) / 10.0)?;
            let (palette, quantized_indexed) = quantization.remapped(&mut source_pixels)?;
            abort_if_untargeted!();

            // Display
            let quantized_rgba = quantized_indexed
                .iter()
                .flat_map(|&i| palette[i as usize].iter())
                .collect::<Vec<_>>();
            let image = RgbImage::new(&quantized_rgba, source.w(), source.h(), Rgba8)?;
            abort_if_untargeted!();
            displayed.replace(working);
            frame.set_image(Some(image));
            frame.redraw();
            app::awake();
        }
    });

    // Run
    tx.send(())?;
    app.run()?;
    Ok(())
}
