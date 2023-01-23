use rgb::{RGB8, RGBA8};
use std::io::Write;

pub struct CachedOption<K: PartialEq, V>(Option<(K, V)>);

impl<K: PartialEq, V> Default for CachedOption<K, V> {
    fn default() -> Self {
        Self(None)
    }
}

impl<K: PartialEq, V> CachedOption<K, V> {
    pub fn get_or_insert_with<F: FnOnce() -> V>(&mut self, key: K, f: F) -> &mut V {
        // Pending https://github.com/rust-lang/rust/issues/93050
        if self.0.as_ref().filter(|&(k, _)| k != &key).is_some() {
            self.0.take();
        }
        &mut self.0.get_or_insert_with(|| (key, f())).1
    }
}

#[derive(Default)]
pub struct CountingSink(usize);

impl CountingSink {
    pub const fn len(&self) -> usize {
        self.0
    }
}

impl Write for CountingSink {
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let size = input.len();
        self.0 += size;
        Ok(size)
    }
}

pub trait RGBAs {
    fn separate_alpha(&self) -> (Vec<RGB8>, Vec<u8>);
    fn without_alpha(&self) -> Vec<RGB8>;
}

impl RGBAs for Vec<RGBA8> {
    fn separate_alpha(&self) -> (Vec<RGB8>, Vec<u8>) {
        self.iter().map(|p| (p.rgb(), p.a)).unzip()
    }

    fn without_alpha(&self) -> Vec<RGB8> {
        self.iter().map(RGBA8::rgb).collect()
    }
}

pub trait RGBs {
    fn with_alpha(&self) -> Vec<RGBA8>;
}

impl<'a> RGBs for &'a [RGB8] {
    fn with_alpha(&self) -> Vec<RGBA8> {
        self.iter().map(|rgb| rgb.alpha(u8::MAX)).collect()
    }
}

// Pending https://github.com/rust-lang/rust/issues/67057
pub fn u8_from_f64(n: f64) -> u8 {
    #[allow(clippy::cast_possible_truncation)]
    (n.round() as i64).try_into().unwrap()
}
