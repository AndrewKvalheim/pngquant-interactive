use std::io::Write;

#[derive(Default)]
pub struct CountingSink(usize);

impl CountingSink {
    pub fn len(&self) -> usize {
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

// Pending https://github.com/rust-lang/rust/issues/67057
pub fn u8_from_f64(n: f64) -> u8 {
    (n.round() as i64).try_into().unwrap()
}
