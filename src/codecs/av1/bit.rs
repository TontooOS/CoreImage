//! Raw bitstream reader implementing the `f(n)`, `su(n)`, `ns(n)`, `le(n)`,
//! `leb128()` and `uvlc()` descriptors of the AV1 specification.

use super::Av1Error;

fn err(m: impl Into<String>) -> Av1Error {
    Av1Error::Decode(m.into())
}

/// MSB first bit reader over a byte slice.
pub struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    /// Current bit position indicator.
    pub fn position(&self) -> usize {
        self.bit_pos
    }

    pub fn set_position(&mut self, bit_pos: usize) {
        self.bit_pos = bit_pos;
    }

    /// Byte position of the next unread bit, rounded down.
    pub fn byte_position(&self) -> usize {
        self.bit_pos / 8
    }

    pub fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.bit_pos
    }

    /// Underlying bytes.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// `f(n)`: unsigned n bit number, high to low order.
    pub fn f(&mut self, n: u32) -> Result<u32, Av1Error> {
        if n == 0 {
            return Ok(0);
        }
        if self.bits_left() < n as usize {
            return Err(err("bitstream underrun"));
        }
        let mut x = 0u32;
        for _ in 0..n {
            let byte = self.data[self.bit_pos >> 3];
            let bit = (byte >> (7 - (self.bit_pos & 7))) & 1;
            x = (x << 1) | bit as u32;
            self.bit_pos += 1;
        }
        Ok(x)
    }

    /// `f(1)` as a flag.
    pub fn flag(&mut self) -> Result<bool, Av1Error> {
        Ok(self.f(1)? == 1)
    }

    /// `su(n)`: signed n bit number.
    pub fn su(&mut self, n: u32) -> Result<i32, Av1Error> {
        if n == 0 {
            return Ok(0);
        }
        let value = self.f(n)?;
        let sign_mask = 1u32 << (n - 1);
        Ok(if value & sign_mask != 0 {
            value as i32 - 2 * sign_mask as i32
        } else {
            value as i32
        })
    }

    /// `ns(n)`: unsigned integer with n possible values, non-symmetric coding.
    pub fn ns(&mut self, n: u32) -> Result<u32, Av1Error> {
        if n <= 1 {
            return Ok(0);
        }
        let w = floor_log2(n) + 1;
        let m = (1u32 << w) - n;
        let v = self.f(w - 1)?;
        if v < m {
            return Ok(v);
        }
        let extra = self.f(1)?;
        Ok((v << 1) - m + extra)
    }

    /// `le(n)`: unsigned little endian n byte number.
    pub fn le(&mut self, n: u32) -> Result<u64, Av1Error> {
        let mut t = 0u64;
        for i in 0..n {
            let byte = self.f(8)?;
            t += (byte as u64) << (i * 8);
        }
        Ok(t)
    }

    /// `leb128()`: variable length unsigned little endian bytes.
    pub fn leb128(&mut self) -> Result<u32, Av1Error> {
        let mut value = 0u32;
        for i in 0..8u32 {
            let byte = self.f(8)?;
            value |= (byte & 0x7f) << (i * 7);
            if byte & 0x80 == 0 {
                break;
            }
        }
        Ok(value)
    }

    /// `uvlc()`: variable length unsigned number.
    pub fn uvlc(&mut self) -> Result<u32, Av1Error> {
        let mut leading_zeros = 0u32;
        loop {
            if self.f(1)? == 1 {
                break;
            }
            leading_zeros += 1;
            if leading_zeros > 32 {
                return Ok(u32::MAX);
            }
        }
        if leading_zeros >= 32 {
            return Ok(u32::MAX);
        }
        let value = self.f(leading_zeros)?;
        Ok(value + (1 << leading_zeros) - 1)
    }

    /// `trailing_bits()`: a single one bit followed by zero padding.
    pub fn trailing_bits(&mut self) -> Result<(), Av1Error> {
        if self.f(1)? != 1 {
            return Err(err("trailing bit pattern without stop bit"));
        }
        while self.bit_pos & 7 != 0 {
            if self.f(1)? != 0 {
                return Err(err("trailing bit pattern with non zero padding"));
            }
        }
        Ok(())
    }

    /// Skip to the next byte boundary, requiring zero padding bits.
    pub fn byte_align(&mut self) -> Result<(), Av1Error> {
        while self.bit_pos & 7 != 0 {
            if self.f(1)? != 0 {
                return Err(err("non zero byte alignment padding"));
            }
        }
        Ok(())
    }
}

/// `FloorLog2( x )` for unsigned values.
pub fn floor_log2(x: u32) -> u32 {
    debug_assert!(x > 0);
    31 - x.leading_zeros()
}

/// `FloorLog2( x )` for signed values, counting the sign bit.
pub fn floor_log2_signed(x: i32) -> u32 {
    if x == 0 {
        return 0;
    }
    let v = if x < 0 { !(x - 1) } else { x } as u32;
    32 - v.leading_zeros()
}

/// `Round2( x, n )`: round to the nearest multiple of `2^n`, ties away from zero.
pub fn round2(x: i64, n: u32) -> i64 {
    if n == 0 {
        return x;
    }
    let bias = 1i64 << (n - 1);
    if x >= 0 {
        (x + bias) >> n
    } else {
        -((-x + bias) >> n)
    }
}

/// `Clip3( low, high, value )`.
pub fn clip3(low: i64, high: i64, value: i64) -> i64 {
    value.max(low).min(high)
}

/// `Abs( x )` for signed values.
pub fn abs_i64(x: i64) -> i64 {
    x.abs()
}