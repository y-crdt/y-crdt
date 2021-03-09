pub const F64_MAX_SAFE_INTEGER: f64 = (i64::pow(2, 53) - 1) as f64;
pub const F64_MIN_SAFE_INTEGER: f64 = -F64_MAX_SAFE_INTEGER;

pub trait Uint: Eq + Ord + Default {
    fn shift6_rest_to_byte(&mut self) -> u8;
    fn shift7_rest_to_byte(&mut self) -> u8;
    fn unshift_add(&mut self, unshift: usize, add: u8);
    fn is_null(&self) -> bool;
}

impl Uint for u32 {
    #[inline]
    fn shift6_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b00111111;
        *self >>= 6;
        b
    }
    #[inline]
    fn shift7_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b01111111;
        *self >>= 7;
        b
    }
    #[inline]
    fn unshift_add(&mut self, unshift: usize, add: u8) {
        *self |= (add as Self) << unshift;
    }
    #[inline]
    fn is_null(&self) -> bool {
        *self == 0
    }
}

impl Uint for u64 {
    #[inline]
    fn shift6_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b00111111;
        *self >>= 6;
        b
    }
    #[inline]
    fn shift7_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b01111111;
        *self >>= 7;
        b
    }
    #[inline]
    fn unshift_add(&mut self, unshift: usize, add: u8) {
        *self |= (add as Self) << unshift;
    }
    #[inline]
    fn is_null(&self) -> bool {
        *self == 0
    }
}

impl Uint for u128 {
    #[inline]
    fn shift6_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b00111111;
        *self >>= 6;
        b
    }
    #[inline]
    fn shift7_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b01111111;
        *self >>= 7;
        b
    }
    #[inline]
    fn unshift_add(&mut self, unshift: usize, add: u8) {
        *self |= (add as Self) << unshift;
    }
    #[inline]
    fn is_null(&self) -> bool {
        *self == 0
    }
}

impl Uint for usize {
    #[inline]
    fn shift6_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b00111111;
        *self >>= 6;
        b
    }
    #[inline]
    fn shift7_rest_to_byte(&mut self) -> u8 {
        let b = *self as u8 & 0b01111111;
        *self >>= 7;
        b
    }
    #[inline]
    fn unshift_add(&mut self, unshift: usize, add: u8) {
        *self |= (add as Self) << unshift;
    }
    #[inline]
    fn is_null(&self) -> bool {
        *self == 0
    }
}
