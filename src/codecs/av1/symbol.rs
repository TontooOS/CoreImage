//! AV1 symbol decoder (MSAC): `init_symbol`, `read_symbol`, `read_literal`
//! and `exit_symbol` as specified in section 8.2 of the AV1 specification.

use super::Av1Error;
use super::bit::{BitReader, floor_log2};

/// Number of probability bits: CDF entries are 15 bit values.
const PROB_BITS: u32 = 15;
/// `EC_MIN_PROB` from the specification.
const MIN_PROB: u32 = super::spec_consts::EC_MIN_PROB as u32;
/// `EC_PROB_SHIFT` from the specification.
const EC_PROB_SHIFT: u32 = super::spec_consts::EC_PROB_SHIFT as u32;

/// Adaptive binary arithmetic decoder over one tile partition.
pub struct SymbolDecoder<'a> {
    reader: BitReader<'a>,
    range: u32,
    value: u32,
    max_bits: i32,
    disable_cdf_update: bool,
}

impl<'a> SymbolDecoder<'a> {
    /// `init_symbol( sz )` for a partition of `sz` bytes.
    pub fn new(data: &'a [u8], disable_cdf_update: bool) -> Result<Self, Av1Error> {
        let sz = data.len();
        let num_bits = ((sz * 8) as u32).min(PROB_BITS);
        let mut reader = BitReader::new(data);
        let buf = reader.f(num_bits)?;
        let padded = buf << (PROB_BITS - num_bits);
        Ok(Self {
            reader,
            range: 1 << PROB_BITS,
            value: ((1 << PROB_BITS) - 1) ^ padded,
            max_bits: (sz * 8) as i32 - PROB_BITS as i32,
            disable_cdf_update,
        })
    }

    /// Bit position of the underlying bitstream, needed by the exit process.
    pub fn position(&self) -> usize {
        self.reader.position()
    }

    /// `read_bool( )`: a pseudo raw bit with equal probability.
    fn read_bool(&mut self) -> Result<u32, Av1Error> {
        // The three entry CDF is rebuilt on every call, so the adapted values
        // are never observed and the update can be skipped.
        let mut cdf = [1u16 << 14, 1u16 << PROB_BITS, 0];
        self.read_symbol_inner(&mut cdf, true).map(|s| s as u32)
    }

    /// `read_literal( n )`, also spelled `L( n )` in the syntax tables.
    pub fn read_literal(&mut self, n: u32) -> Result<u32, Av1Error> {
        let mut x = 0u32;
        for _ in 0..n {
            x = (x << 1) | self.read_bool()?;
        }
        Ok(x)
    }

    /// `NS( n )`: unsigned arithmetic coded integer with n possible values.
    pub fn read_ns(&mut self, n: u32) -> Result<u32, Av1Error> {
        if n <= 1 {
            return Ok(0);
        }
        let w = floor_log2(n) + 1;
        let m = (1u32 << w) - n;
        let v = self.read_literal(w - 1)?;
        if v < m {
            return Ok(v);
        }
        let extra = self.read_literal(1)?;
        Ok((v << 1) - m + extra)
    }

    /// `read_symbol( cdf )`: decode one symbol and adapt the distribution.
    pub fn read_symbol(&mut self, cdf: &mut [u16]) -> Result<usize, Av1Error> {
        self.read_symbol_inner(cdf, false)
    }

    fn read_symbol_inner(&mut self, cdf: &mut [u16], skip_update: bool) -> Result<usize, Av1Error> {
        let n = cdf.len() - 1;
        debug_assert!(n > 0);
        let mut symbol: usize = 0;
        let mut cur;
        let mut prev = self.range;
        loop {
            let f = (1u32 << PROB_BITS) - cdf[symbol] as u32;
            cur = ((self.range >> 8) * (f >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT);
            cur += MIN_PROB * (n - symbol - 1) as u32;
            symbol += 1;
            if self.value >= cur {
                break;
            }
            prev = cur;
        }
        symbol -= 1;
        self.range = prev - cur;
        self.value -= cur;
        // Renormalise.
        let bits = PROB_BITS - floor_log2(self.range);
        self.range <<= bits;
        let num_bits = bits.min(self.max_bits.max(0) as u32);
        let new_data = self.reader.f(num_bits).unwrap_or(0);
        let padded = new_data << (bits - num_bits);
        self.value = padded ^ (((self.value + 1) << bits) - 1);
        self.max_bits -= bits as i32;
        if !skip_update && !self.disable_cdf_update {
            let rate = 3
                + u32::from(cdf[n] > 15)
                + u32::from(cdf[n] > 31)
                + floor_log2(n as u32).min(2);
            let mut tmp = 0u16;
            for i in 0..n - 1 {
                if i == symbol {
                    tmp = 1 << PROB_BITS;
                }
                if tmp < cdf[i] {
                    cdf[i] -= (cdf[i] - tmp) >> rate;
                } else {
                    cdf[i] += (tmp - cdf[i]) >> rate;
                }
            }
            cdf[n] += u16::from(cdf[n] < 32);
        }
        Ok(symbol)
    }

    /// `exit_symbol( )`: consume the trailing bits of the partition.
    pub fn exit(&mut self) -> Result<(), Av1Error> {
        let trailing_bits = 15i32 - (self.max_bits + 15).min(15);
        let trailing_bit_position = self.reader.position() as i64 - trailing_bits as i64;
        let skip = self.max_bits.max(0) as usize;
        self.reader.set_position(self.reader.position() + skip);
        if trailing_bit_position >= 0 {
            let bit = self.peek_bit(trailing_bit_position as usize);
            if bit != Some(1) {
                return Err(Av1Error::Decode("tile trailing bit is not one".into()));
            }
        }
        Ok(())
    }

    fn peek_bit(&self, bit_pos: usize) -> Option<u8> {
        let byte = self.reader.data().get(bit_pos >> 3)?;
        Some((byte >> (7 - (bit_pos & 7))) & 1)
    }

    /// CDF update rate helper used by the frame end update process.
    pub fn disable_cdf_update(&self) -> bool {
        self.disable_cdf_update
    }
}