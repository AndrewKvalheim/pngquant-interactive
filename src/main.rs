mod utils;

use crate::utils::{u8_from_f64, CountingSink};
use anyhow::Result;
use clap::Parser;
use fltk::{
    app::{self, App, Scheme},
    enums::ColorDepth::Rgba8,
    frame::Frame,
    image::{PngImage, RgbImage},
    misc::Progress,
    prelude::*,
    valuator::HorValueSlider,
    window::Window,
};
use fltk_theme::{color_themes, ColorTheme};
use png::{ColorType, Compression, Encoder};
use rgb::{AsPixels, ComponentBytes, RGBA8};
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
    let (m, gh, sh) = (8, 12, 24);
    let mut y = 0;
    let app = App::default().with_scheme(Scheme::Gtk);
    ColorTheme::new(color_themes::DARK_THEME).apply();
    let mut window = Window::default();
    let mut preview = Frame::default().with_pos(0, 0).with_size(vw, y + vh);
    y += vh;
    let mut gauge = Progress::default()
        .with_pos(m, y + m)
        .with_size(vw - m * 2, 12);
    gauge.set_minimum(0.0);
    gauge.set_maximum(1.0);
    gauge.set_value(0.0);
    y += m + gh + m;
    let mut lh = 0;
    macro_rules! slider {
        ($l:expr, $param:ident, $min:expr, $max:expr, $c0:expr, $c1:expr) => {{
            let (c, tx, params) = (4, tx.clone(), params.clone());
            let mut slider = HorValueSlider::default()
                .with_pos((vw - m) / c * $c0 + m, y)
                .with_size((vw - m) / c * $c1 - (vw - m) / c * $c0 - m, sh)
                .with_label($l);
            lh = lh.max(slider.measure_label().1);
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
    y += sh + lh + m;
    window.set_size(vw, y);
    window.end();
    window.show();

    // Initialize quantizer
    let source_rgba = source.to_rgb_data();
    let mut quantizer = imagequant::new();

    // Initialize worker
    thread::spawn(move || -> Result<()> {
        let mut calibrated_gauge = false;
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
            preview.set_image(Some(image));
            preview.redraw();
            app::awake();

            // Estimate size
            let estimate = |palette: Option<&[RGBA8]>, data: &[u8]| -> Result<usize> {
                let mut sink = CountingSink::default();
                {
                    let mut encoder = Encoder::new(&mut sink, w.try_into()?, h.try_into()?);
                    encoder.set_compression(Compression::Fast);
                    if let Some(p) = palette {
                        encoder.set_color(ColorType::Indexed);
                        encoder.set_palette(p.as_bytes());
                    } else {
                        encoder.set_color(ColorType::Rgba);
                    }
                    let mut writer = encoder.write_header()?;
                    writer.write_image_data(data)?;
                }
                Ok(sink.len())
            };
            if !calibrated_gauge {
                gauge.set_maximum(estimate(None, &source_rgba)? as f64);
                calibrated_gauge = true;
                abort_if_untargeted!();
            }
            gauge.set_value(estimate(Some(&palette), &quantized_indexed)? as f64);
            gauge.redraw();
            app::awake();

            displayed.replace(working);
        }
    });

    // Run
    tx.send(())?;
    app.run()?;
    Ok(())
}
