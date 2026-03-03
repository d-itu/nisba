use core::{marker::PhantomData, result};

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Container length overflow")]
    ContainerLengthOverflow,
}

pub type Result<T> = result::Result<T, Error>;

pub struct Encoder<'a> {
    ptr: *mut u8,
    marker: PhantomData<&'a [u8]>,
}

impl<'a> Encoder<'a> {
    pub const fn new(ptr: *mut u8) -> Self {
        Self {
            ptr,
            marker: PhantomData,
        }
    }
    pub const unsafe fn push_u8(&mut self, byte: u8) {
        unsafe {
            *self.ptr = byte;
            self.ptr = self.ptr.add(1);
        }
    }
    pub const unsafe fn push_bytes(&mut self, bytes: &[u8]) {
        unsafe {
            bytes.as_ptr().copy_to_nonoverlapping(self.ptr, bytes.len());
            self.ptr = self.ptr.add(bytes.len());
        }
    }
    pub const unsafe fn push_varint_unsigned(&mut self, mut value: u64) {
        while value >= 0x80 {
            unsafe { self.push_u8((value & 0x7f) as u8 | 0x80) };
            value >>= 7;
        }
        unsafe { self.push_u8(value as u8) };
    }
    pub const unsafe fn push_varint_signed(&mut self, value: i64) {
        unsafe { self.push_varint_unsigned(zigzag(value)) };
    }
    pub unsafe fn push(&mut self, value: &impl Encode) {
        unsafe { value.encode(self) }
    }
}

pub unsafe trait Encode {
    fn prepare(&self) -> Result<usize>;
    unsafe fn encode(&self, w: &mut Encoder);
}

const fn zigzag(value: i64) -> u64 {
    let res = (value << 1) ^ (value >> 63);
    res.cast_unsigned()
}

pub const fn varint_calc_size_unsigned(value: u64) -> usize {
    const fn max(x: u32, y: u32) -> u32 {
        if x > y { x } else { y }
    }
    max((64 - value.leading_zeros() + 6) / 7, 1) as _
}

pub const fn varint_calc_size_signed(value: i64) -> usize {
    varint_calc_size_unsigned(zigzag(value))
}

#[cfg(feature = "alloc")]
pub fn encode<T: Encode>(value: &T) -> Result<Box<[u8]>> {
    let len = value.prepare()?;
    let mut buf = Box::new_uninit_slice(len);
    let mut w = Encoder::new(buf.as_mut_ptr().cast());

    unsafe {
        value.encode(&mut w);
        Ok(buf.assume_init())
    }
}
