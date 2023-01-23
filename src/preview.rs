use crate::encode::{Encode, Priority};
use crate::source::Source;
use crate::utilities::{CachedOption, RGBAs};
use anyhow::Result;
use fltk::enums::ColorDepth::Rgba8;
use fltk::image::RgbImage;
use fltk::prelude::ImageExt;
use imagequant::{Attributes, QuantizationResult};
use png::{ColorType, Encoder};
use rgb::{ComponentBytes, RGBA8};
use std::io::Write;

#[derive(Clone, PartialEq)]
pub struct Params {
    pub dithering: u8,
    pub effort: u8,
    pub preservation: u8,
}

pub struct Preview {
    pub source: Source,
    quantizer: Attributes,
    quantization: CachedOption<(u8, u8), QuantizationResult>,
    palette_rgba: Option<Vec<RGBA8>>,
    quantized_indexed: Option<Vec<u8>>,
    quantized_rgba: Option<Vec<u8>>,
}

impl Preview {
    pub fn display(&mut self, width: usize, height: usize) -> Result<RgbImage> {
        let quantized_rgba = self.quantized_rgba.get_or_insert_with(|| {
            let palette = self.palette_rgba.as_ref().expect("quantized");
            let indices = self.quantized_indexed.as_ref().expect("quantized");
            indices
                .iter()
                .flat_map(|&i| palette[usize::from(i)].iter())
                .collect()
        });

        let mut image = RgbImage::new(
            quantized_rgba,
            self.source.width.try_into()?,
            self.source.height.try_into()?,
            Rgba8,
        )?;

        if width < self.source.width || height < self.source.height {
            image.scale(width.try_into()?, height.try_into()?, true, false);
        };

        Ok(image)
    }

    pub fn quantize(&mut self, params: &Params) -> Result<()> {
        let mut image = self.quantizer.new_image_borrowed(
            &self.source.rgba,
            self.source.width,
            self.source.height,
            0.0,
        )?;

        let (e, p) = (params.effort, params.preservation);
        let quantization = self.quantization.get_or_insert_with((e, p), || {
            self.quantizer.set_speed(11 - i32::from(e)).unwrap();
            self.quantizer.set_quality(0, p).unwrap();
            self.quantizer.quantize(&mut image).unwrap()
        });

        quantization.set_dithering_level(f32::from(params.dithering) / 10.0)?;
        let (palette_rgba, quantized_indexed) = quantization.remapped(&mut image)?;

        self.quantized_rgba.take();
        self.palette_rgba.replace(palette_rgba);
        self.quantized_indexed.replace(quantized_indexed);
        Ok(())
    }
}

impl Encode for Preview {
    fn encode<W: Write>(&self, priority: Priority, into: W) -> Result<()> {
        let Source { width, height, .. } = self.source;
        let mut encoder = Encoder::new(into, width.try_into()?, height.try_into()?);
        encoder.set_compression(priority.into());
        encoder.set_color(ColorType::Indexed);

        let palette_rgba = self.palette_rgba.as_ref().expect("quantized");
        let palette_rgb = if self.source.uses_alpha {
            let (rgb, a) = palette_rgba.separate_alpha();
            encoder.set_trns(a);
            rgb
        } else {
            palette_rgba.without_alpha()
        };
        encoder.set_palette(palette_rgb.as_bytes());

        Ok(encoder
            .write_header()?
            .write_image_data(self.quantized_indexed.as_ref().expect("quantized"))?)
    }
}

impl From<Source> for Preview {
    fn from(source: Source) -> Self {
        Self {
            source,
            quantizer: imagequant::new(),
            quantization: CachedOption::default(),
            palette_rgba: None,
            quantized_indexed: None,
            quantized_rgba: None,
        }
    }
}
