use crate::encode::{Encode, Priority};
use crate::utilities::RGBs;
use anyhow::Result;
use fltk::enums::ColorDepth::{Rgb8, Rgba8};
use fltk::prelude::ImageExt;
use png::{ColorType, Encoder};
use rgb::{ComponentBytes, FromSlice, RGBA8};
use std::io::Write;

pub struct Source {
    pub uses_alpha: bool,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<RGBA8>,
}

impl Encode for Source {
    fn encode<W: Write>(&self, priority: Priority, into: W) -> Result<()> {
        let mut encoder = Encoder::new(into, self.width.try_into()?, self.height.try_into()?);
        encoder.set_compression(priority.into());
        encoder.set_color(ColorType::Rgba);

        Ok(encoder
            .write_header()?
            .write_image_data(self.rgba.as_bytes())?)
    }
}

impl<I: ImageExt> From<I> for Source {
    #[allow(clippy::cast_sign_loss)]
    fn from(image: I) -> Self {
        match image.depth() {
            Rgb8 => Self {
                uses_alpha: false,
                width: image.width() as usize,
                height: image.height() as usize,
                rgba: image.to_rgb_data().as_rgb().with_alpha(),
            },
            Rgba8 => Self {
                uses_alpha: true,
                width: image.width() as usize,
                height: image.height() as usize,
                rgba: image.to_rgb_data().as_rgba().to_owned(),
            },
            d => unimplemented!("color mode {:?}", d),
        }
    }
}
