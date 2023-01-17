use anyhow::Result;
use clap::Parser;
use fltk::{
    app::{self, App, Scheme},
    enums::ColorDepth::Rgba8,
    frame::Frame,
    image::{PngImage, RgbImage},
    prelude::*,
    valuator::HorValueSlider,
    window::Window,
};
use fltk_theme::{color_themes, ColorTheme};
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
    pub effort: u8,
    pub quality: u8,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            dithering: 0,
            effort: 10,
            quality: 20,
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

    // Load source
    let source = PngImage::load(args.path)?;
    if source.depth() != Rgba8 {
        unimplemented!("color mode {:?}", source.depth())
    }
    let (w, h) = (source.width(), source.height());
    let (vw, vh) = (w.max(480), h);

    // Initialize GUI
    let app = App::default().with_scheme(Scheme::Gtk);
    ColorTheme::new(color_themes::DARK_THEME).apply();
    let mut window = Window::default().with_size(vw, vh);
    let mut frame = Frame::default().with_pos(0, 0).with_size(vw, vh);
    macro_rules! slider {
        ($l:expr, $param:ident, $min:expr, $max:expr, $c0:expr, $c1:expr) => {{
            let (c, m, sh, tx, params) = (4, 8, 24, tx.clone(), params.clone());
            let mut slider = HorValueSlider::default()
                .with_pos((vw - m) / c * $c0 + m, vh + m)
                .with_size((vw - m) / c * $c1 - (vw - m) / c * $c0 - m, sh)
                .with_label($l);
            let (_, lh) = slider.measure_label();
            window.set_size(window.width(), window.height().max(vh + m + sh + lh + m));
            slider.set_minimum($min.into());
            slider.set_maximum($max.into());
            slider.set_step(1.0, 1);
            slider.set_value(params.read().unwrap().$param.into());
            slider.set_callback(move |s| {
                params.write().unwrap().$param = u8_from_f64(s.value());
                tx.send(()).unwrap();
            });
            slider
        }};
    }
    slider!("Effort", effort, 1, 10, 0, 1);
    slider!("Color Preservation", quality, 0, 100, 1, 3).take_focus()?;
    slider!("Dithering", dithering, 0, 10, 3, 4);
    window.end();
    window.show();

    // Initialize quantizer
    let source_rgba = source.to_rgb_data();
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
            quantizer.set_speed(11 - i32::from(working.effort))?;
            let mut source_pixels = quantizer.new_image_borrowed(
                source_rgba.as_pixels(),
                w.try_into()?,
                h.try_into()?,
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
                .flat_map(|&i| palette[usize::from(i)].iter())
                .collect::<Vec<_>>();
            let image = RgbImage::new(&quantized_rgba, w, h, Rgba8)?;
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
