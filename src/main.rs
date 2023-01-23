#![warn(clippy::nursery, clippy::pedantic)]
#![allow(
    clippy::derive_partial_eq_without_eq,
    clippy::similar_names,
    clippy::too_many_lines
)]

mod encode;
mod preview;
mod source;
mod utilities;

use crate::encode::{Encode, Priority};
use crate::preview::{Params, Preview};
use crate::source::Source;
use crate::utilities::u8_from_f64;
use anyhow::Result;
use clap::{value_parser, Parser};
use fltk::app::{self, App, Scheme};
use fltk::button::Button;
use fltk::enums::{Color, Event as UiEvent, Key};
use fltk::frame::Frame;
use fltk::image::PngImage;
use fltk::misc::Progress;
use fltk::prelude::*;
use fltk::valuator::HorValueSlider;
use fltk::window::Window;
use fltk_theme::{color_themes, ColorTheme};
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

fn main() -> Result<()> {
    let args = Args::parse();
    let source = Source::from(PngImage::load(&args.path)?);
    let params = Arc::new(RwLock::new(Params {
        dithering: args.dithering,
        effort: args.effort,
        preservation: args.preservation,
    }));

    let (to_app, for_app) = app::channel();
    let (to_worker, for_worker) = mpsc::channel();

    // Build GUI
    let (c, m, lh, gh, sh) = (8, 8, 20, 12, 24);
    let (ww_min, wh_min) = (480, m + gh + m + sh + lh + m);
    let (vw, vh) = (
        (i32::try_from(source.width)?).max(ww_min),
        i32::try_from(source.height)?,
    );
    let (wh, cw) = (vh + m + gh + m + sh + lh + m, (vw - m) / c);
    let app = App::default().with_scheme(Scheme::Gtk);
    ColorTheme::new(color_themes::DARK_THEME).apply();
    let mut window = Window::default().with_size(vw, wh).with_label(&format!(
        "{} · pngquant-interactive",
        &args.path.file_name().expect("file").to_str().expect("UTF8")
    ));
    let mut view = Frame::default().with_pos(0, 0).with_size(vw, vh);
    let mut spinner = Frame::default()
        .with_pos(0, 0)
        .with_size(vw, vh)
        .with_label("@refresh");
    let mut gauge = Progress::default()
        .with_pos(m, vh + m)
        .with_size(vw - m * 2, gh);
    gauge.set_selection_color(Color::Foreground);
    gauge.set_minimum(0.0);
    gauge.set_maximum(1.0);
    gauge.set_value(0.0);
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
            slider.set_value(params.read().expect("params").$param.into());
            slider.set_callback(move |s| {
                params.write().expect("params").$param = u8_from_f64(s.value());
                to_worker.send(Action::Preview).expect("worker");
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
            b.window().expect("window").deactivate();
            to_worker.send(Action::Export).expect("worker");
        }
    });
    window.resizable(&view);
    window.size_range(ww_min, wh_min, 0, 0);
    window.handle({
        let to_worker = to_worker.clone();
        move |_, event| match event {
            UiEvent::KeyDown if app::event_key() == Key::Enter => {
                ok_button.do_callback();
                true
            }
            UiEvent::Resize => {
                to_worker.send(Action::Resize).expect("worker");
                false
            }
            _ => false,
        }
    });
    window.end();
    window.show();

    // Start worker
    thread::spawn(move || -> Result<()> {
        let mut preview = Preview::from(source);
        let mut viewed_params = None;
        let mut viewed_size = None;

        #[allow(clippy::cast_precision_loss)]
        gauge.set_maximum(preview.source.estimate()? as f64);

        loop {
            match for_worker.recv()? {
                Action::Export => {
                    let params = params.read().expect("params").clone();
                    let path = args.path.with_file_name(format!(
                        "{}-{}.png",
                        args.path.file_stem().expect("file").to_str().expect("UTF8"),
                        if params.dithering == 0 { "or8" } else { "fs8" }
                    ));

                    preview.quantize(&params)?;
                    preview.encode(Priority::Size, BufWriter::new(File::create(path)?))?;
                    to_app.send(Event::Exported);
                }
                Action::Preview => {
                    let working = params.read().expect("params").clone();
                    macro_rules! abort_if_untargeted {
                        () => {
                            if *params.read().expect("params") != working {
                                continue;
                            }
                        };
                    }
                    match &viewed_params {
                        Some(p) if p == &working => continue,
                        _ => {}
                    }
                    spinner.show();

                    // Quantize
                    preview.quantize(&working)?;
                    abort_if_untargeted!();

                    // Display
                    #[allow(clippy::cast_sign_loss)]
                    let (width, height) = (view.width() as usize, view.height() as usize);
                    let image = preview.display(width, height)?;
                    abort_if_untargeted!();
                    view.set_image(Some(image));
                    viewed_size.replace((width, height));
                    spinner.hide();
                    app::awake();

                    // Estimate size
                    let estimate = preview.estimate()?;
                    abort_if_untargeted!();
                    #[allow(clippy::cast_precision_loss)]
                    gauge.set_value(estimate as f64);
                    viewed_params.replace(working);
                    gauge.redraw();
                    app::awake();
                }
                Action::Resize => {
                    if let Some((pvw, pvh)) = viewed_size {
                        #[allow(clippy::cast_sign_loss)]
                        let (vw, vh) = (view.width() as usize, view.height() as usize);
                        let (w, h) = (preview.source.width, preview.source.height);

                        if vw < pvw || vh < pvh || (pvw < vw && pvw < w) || (pvh < vh && pvh < h) {
                            view.set_image(Some(preview.display(vw, vh)?));
                            viewed_size.replace((vw, vh));
                            spinner.show(); // Workaround to fully redraw view
                            spinner.hide();
                            app::awake();
                        }
                    }
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
