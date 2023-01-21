mod utils;

use crate::utils::{u8_from_f64, CountingSink};
use anyhow::Result;
use clap::{value_parser, Parser};
use fltk::{
    app::{self, App, Scheme},
    button::Button,
    enums::{
        Color,
        ColorDepth::{Rgb8, Rgba8},
        Event as UiEvent, Key,
    },
    frame::Frame,
    image::{PngImage, RgbImage},
    misc::Progress,
    prelude::*,
    valuator::HorValueSlider,
    window::Window,
};
use fltk_theme::{color_themes, ColorTheme};
use png::{ColorType, Compression, Encoder};
use rgb::{ComponentBytes, FromSlice, RGBA8};
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, RwLock};
use std::thread;

#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// Speed–quality tradeoff (speed <11−E>) 1–10
    #[arg(long, short, value_name = "E", default_value_t = 10, value_parser = value_parser!(u8).range(1..=10))]
    effort: u8,

    /// Color preservation cutoff (quality 0-<P>) 0–100
    #[arg(long, short, value_name = "P", default_value_t = 50, value_parser = value_parser!(u8).range(0..=100))]
    preservation: u8,

    /// Amount of dithering (floyd <D∕10>) 0–10
    #[arg(long, short, value_name = "D", default_value_t = 0, value_parser = value_parser!(u8).range(0..=10))]
    dithering: u8,

    /// Source PNG file
    #[arg()]
    path: PathBuf,
}

enum Action {
    Export,
    Preview,
    Resize,
}

enum Event {
    Exported,
}

#[derive(Clone, Debug, PartialEq)]
struct Params {
    dithering: u8,
    effort: u8,
    preservation: u8,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let params = Arc::new(RwLock::new(Params {
        dithering: args.dithering,
        effort: args.effort,
        preservation: args.preservation,
    }));
    let (to_app, for_app) = app::channel();
    let (to_worker, for_worker) = mpsc::channel();

    // Load source
    let source = PngImage::load(&args.path)?;
    let (w, h) = (source.width(), source.height());

    // Initialize GUI
    let (c, m, lh, gh, sh) = (8, 8, 20, 12, 24);
    let (wwm, whm) = (480, m + gh + m + sh + lh + m);
    let (vw, vh) = (w.max(wwm), h);
    let (wh, cw) = (vh + m + gh + m + sh + lh + m, (vw - m) / c);
    let app = App::default().with_scheme(Scheme::Gtk);
    ColorTheme::new(color_themes::DARK_THEME).apply();
    let mut window = Window::default().with_size(vw, wh).with_label(&format!(
        "{} · pngquant",
        &args.path.file_name().unwrap().to_str().unwrap()
    ));
    let mut preview = Frame::default().with_pos(0, 0).with_size(vw, vh);
    let mut spinner = Frame::default().with_pos(0, 0).with_size(vw, vh);
    spinner.set_label("@refresh");
    let mut gauge = Progress::default()
        .with_pos(m, vh + m)
        .with_size(vw - m * 2, gh);
    gauge.set_minimum(0.0);
    gauge.set_maximum(1.0);
    gauge.set_value(0.0);
    gauge.set_selection_color(Color::Foreground);
    macro_rules! slider {
        ($l:expr, $param:ident, $min:expr, $max:expr, $c0:expr, $c1:expr) => {{
            let (to_worker, params) = (to_worker.clone(), params.clone());
            let mut slider = HorValueSlider::default()
                .with_pos(cw * $c0 + m, vh + m + gh + m)
                .with_size(cw * $c1 - cw * $c0 - m, sh)
                .with_label($l);
            slider.set_minimum($min.into());
            slider.set_maximum($max.into());
            slider.set_step(1.0, 1);
            slider.set_value(params.read().unwrap().$param.into());
            slider.set_callback(move |s| {
                params.write().unwrap().$param = u8_from_f64(s.value());
                to_worker.send(Action::Preview).unwrap();
            });
            slider
        }};
    }
    slider!("Effort", effort, 1, 10, 0, 2);
    slider!("Color Preservation", preservation, 0, 100, 2, 5).take_focus()?;
    slider!("Dithering", dithering, 0, 10, 5, 7);
    let mut ok_button = Button::default()
        .with_pos(cw * 7 + m, vh + m + gh + m)
        .with_size(cw * 8 - cw * 7 - m, sh + lh)
        .with_label("OK");
    ok_button.set_callback({
        let to_worker = to_worker.clone();
        move |b| {
            b.window().unwrap().deactivate();
            to_worker.send(Action::Export).unwrap();
        }
    });
    window.handle({
        let to_worker = to_worker.clone();
        move |_, event| match event {
            UiEvent::KeyDown if app::event_key() == Key::Enter => {
                ok_button.do_callback();
                true
            }
            UiEvent::Resize => {
                to_worker.send(Action::Resize).unwrap();
                true
            }
            _ => false,
        }
    });
    window.resizable(&preview);
    window.size_range(wwm, whm, 0, 0);
    window.end();
    window.show();

    // Initialize quantizer
    let (source_rgba, uses_alpha) = match source.depth() {
        Rgb8 => (
            source
                .to_rgb_data()
                .as_rgb()
                .iter()
                .map(|rgb| rgb.alpha(u8::MAX))
                .collect(),
            false,
        ),
        Rgba8 => (source.to_rgb_data().as_rgba().to_owned(), true),
        d => unimplemented!("color mode {:?}", d),
    };
    let mut quantizer = imagequant::new();

    // Initialize worker
    thread::spawn(move || -> Result<()> {
        let mut calibrated_gauge = false;
        let mut displayed_params = None;
        let mut displayed_size = None;
        let mut quantized_rgba: Vec<u8> = Vec::with_capacity(4usize * (w * h) as usize);

        loop {
            match for_worker.recv()? {
                Action::Preview => {
                    let working = params.read().unwrap().clone();
                    match &displayed_params {
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
                    spinner.show();

                    // Quantize
                    quantizer.set_quality(0, working.preservation)?;
                    quantizer.set_speed(11 - i32::from(working.effort))?;
                    let mut source_pixels = quantizer.new_image_borrowed(
                        &source_rgba,
                        w.try_into()?,
                        h.try_into()?,
                        0.0,
                    )?;
                    abort_if_untargeted!();
                    let mut quantization = quantizer.quantize(&mut source_pixels)?;
                    abort_if_untargeted!();
                    quantization.set_dithering_level(f32::from(working.dithering) / 10.0)?;
                    let (palette_rgba, quantized_indexed) =
                        quantization.remapped(&mut source_pixels)?;
                    abort_if_untargeted!();

                    // Display
                    quantized_rgba.clear();
                    quantized_rgba.extend(
                        quantized_indexed
                            .iter()
                            .flat_map(|&i| palette_rgba[usize::from(i)].iter()),
                    );
                    abort_if_untargeted!();
                    let mut image = RgbImage::new(&quantized_rgba, w, h, Rgba8)?;
                    let (pw, ph) = (preview.width(), preview.height());
                    let display_size = if pw < w || ph < h {
                        image.scale(pw, ph, true, false);
                        (pw, ph)
                    } else {
                        (w, h)
                    };
                    abort_if_untargeted!();
                    preview.set_image(Some(image));
                    preview.redraw();
                    app::awake();

                    // Estimate size
                    let estimate = |palette_rgba: Option<&[RGBA8]>, data: &[u8]| -> Result<usize> {
                        let mut sink = CountingSink::default();
                        {
                            let mut encoder = Encoder::new(&mut sink, w.try_into()?, h.try_into()?);
                            encoder.set_compression(Compression::Fast);
                            if let Some(p) = palette_rgba {
                                let (palette_rgb, palette_a): (Vec<_>, Vec<_>) =
                                    p.iter().map(|p| (p.rgb(), p.a)).unzip();
                                encoder.set_color(ColorType::Indexed);
                                encoder.set_palette(palette_rgb.as_bytes());
                                if uses_alpha {
                                    encoder.set_trns(palette_a);
                                }
                                let mut writer = encoder.write_header()?;
                                writer.write_image_data(data)?;
                            } else {
                                encoder.set_color(ColorType::Rgba);
                                let mut writer = encoder.write_header()?;
                                writer.write_image_data(data)?;
                            }
                        }
                        Ok(sink.len())
                    };
                    if !calibrated_gauge {
                        gauge.set_maximum(estimate(None, source_rgba.as_bytes())? as f64);
                        calibrated_gauge = true;
                        abort_if_untargeted!();
                    }
                    gauge.set_value(estimate(Some(&palette_rgba), &quantized_indexed)? as f64);
                    gauge.redraw();
                    spinner.hide();
                    app::awake();

                    displayed_params.replace(working);
                    displayed_size.replace(display_size);
                }
                Action::Resize => {
                    if let Some((dw, dh)) = displayed_size {
                        let (pw, ph) = (preview.width(), preview.height());

                        if pw < dw || (dw < pw && dw < w) || ph < dh || (dh < ph && dh < h) {
                            let mut image = RgbImage::new(&quantized_rgba, w, h, Rgba8)?;
                            let display_size = if pw < w || ph < h {
                                image.scale(pw, ph, true, false);
                                (pw, ph)
                            } else {
                                (w, h)
                            };
                            preview.set_image(Some(image));
                            displayed_size.replace(display_size);
                        }
                    }
                }
                Action::Export => {
                    let working = params.read().unwrap().clone();

                    // Quantize
                    quantizer.set_quality(0, working.preservation)?;
                    quantizer.set_speed(11 - i32::from(working.effort))?;
                    let mut source_pixels = quantizer.new_image_borrowed(
                        &source_rgba,
                        w.try_into()?,
                        h.try_into()?,
                        0.0,
                    )?;
                    let mut quantization = quantizer.quantize(&mut source_pixels)?;
                    quantization.set_dithering_level(f32::from(working.dithering) / 10.0)?;
                    let (palette_rgba, quantized_indexed) =
                        quantization.remapped(&mut source_pixels)?;

                    // Encode
                    let (palette_rgb, palette_a): (Vec<_>, Vec<_>) =
                        palette_rgba.iter().map(|p| (p.rgb(), p.a)).unzip();
                    let path = args.path.with_file_name(format!(
                        "{}-{}.png",
                        args.path.file_stem().unwrap().to_str().unwrap(),
                        if working.dithering == 0 { "or8" } else { "fs8" }
                    ));
                    let file = BufWriter::new(File::create(path)?);
                    let mut encoder = Encoder::new(file, w.try_into()?, h.try_into()?);
                    encoder.set_compression(Compression::Best);
                    encoder.set_color(ColorType::Indexed);
                    encoder.set_palette(palette_rgb.as_bytes());
                    if uses_alpha {
                        encoder.set_trns(palette_a);
                    }
                    let mut writer = encoder.write_header()?;
                    writer.write_image_data(&quantized_indexed)?;

                    to_app.send(Event::Exported);
                }
            }
        }
    });

    // Run
    to_worker.send(Action::Preview)?;
    while app.wait() {
        if let Some(event) = for_app.recv() {
            match event {
                Event::Exported => app.quit(),
            }
        }
    }
    Ok(())
}
