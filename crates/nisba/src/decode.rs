use core::{marker::PhantomData, mem, result, slice};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Unexpected end of input")]
    UnexpectedEnd,
    #[error("Varint overflow")]
    VarintOverflow,
    #[error("Enum {name} has Unknown Discriminant {value}")]
    UnknownDiscriminant { name: &'static str, value: u64 },
}

pub type Result<T> = result::Result<T, Error>;

#[derive(Clone, Copy)]
pub struct Decoder<'a> {
    ptr: *const u8,
    end: *const u8,
    marker: PhantomData<&'a [u8]>,
}

impl<'a> Decoder<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self {
            ptr: bytes.as_ptr(),
            end: unsafe { bytes.as_ptr().add(bytes.len()) },
            marker: PhantomData,
        }
    }
    pub const fn remaining_len(&self) -> usize {
        unsafe { self.end.offset_from_unsigned(self.ptr) }
    }
    pub const fn remaining(&self) -> &'a [u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.remaining_len()) }
    }
    pub const fn next_u8(&mut self) -> Result<u8> {
        if self.remaining_len() == 0 {
            return Err(Error::UnexpectedEnd);
        }
        let byte = unsafe { *self.ptr };
        self.ptr = unsafe { self.ptr.add(1) };
        Ok(byte)
    }
    pub const fn next_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.remaining_len() < len {
            return Err(Error::UnexpectedEnd);
        }
        let bytes = unsafe { slice::from_raw_parts(self.ptr, len) };
        self.ptr = unsafe { self.ptr.add(len) };
        Ok(bytes)
    }
    pub const fn next_varint_unsigned(&mut self, bit_width: usize) -> Result<u64> {
        let mut value = 0;
        let mut shift = 0;
        loop {
            let byte = const_try!(self.next_u8());
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            if shift > bit_width {
                return Err(Error::VarintOverflow);
            }
            shift += 7;
        }
        Ok(value)
    }
    pub const fn next_varint_signed(&mut self, bit_width: usize) -> Result<i64> {
        let n = const_try!(self.next_varint_unsigned(bit_width));
        Ok((n >> 1).cast_signed() ^ -((n & 1).cast_signed()))
    }
    pub fn next<T: Decode<'a>>(&mut self) -> Result<T> {
        T::decode(self)
    }
}

pub unsafe trait Decode<'a>: Sized {
    fn decode(r: &mut Decoder<'a>) -> Result<Self>;
}

pub struct Vector<'a, T> {
    ptr: *const u8,
    len: usize,
    marker: PhantomData<&'a [T]>,
}

impl<'a, T> Vector<'a, T> {
    pub const fn new(data: &'a [u8]) -> Self {
        Self {
            ptr: data.as_ptr(),
            len: data.len(),
            marker: PhantomData,
        }
    }
    pub const fn get(&self, index: usize) -> Option<T> {
        if index >= self.len {
            return None;
        }
        Some(unsafe { self.get_unchecked(index) })
    }
    pub const unsafe fn get_unchecked(&self, index: usize) -> T {
        let byte_index = index * mem::size_of::<T>();
        let ptr: *const T = unsafe { self.ptr.add(byte_index) }.cast();
        unsafe { ptr.read_unaligned() }
    }
    pub const fn next(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let ptr: *const T = self.ptr.cast();
        let result = unsafe { ptr.read_unaligned() };
        self.ptr = unsafe { self.ptr.add(mem::size_of::<T>()) };
        self.len -= 1;
        Some(result)
    }
}

impl<'a, T> Iterator for Vector<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }
}

pub struct Stream<'a, T> {
    decoder: Decoder<'a>,
    marker: PhantomData<T>,
}

impl<'a, T> Stream<'a, T> {
    pub const fn new(data: &'a [u8]) -> Self {
        Self {
            decoder: Decoder::new(data),
            marker: PhantomData,
        }
    }
}

impl<'a, T: Decode<'a>> Iterator for Stream<'a, T> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.decoder.remaining_len() == 0 {
            return None;
        }
        Some(self.decoder.next())
    }
}
