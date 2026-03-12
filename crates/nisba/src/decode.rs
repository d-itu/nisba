use core::{fmt::Debug, marker::PhantomData, mem, result, slice};

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
    pub const fn next_u8_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        unsafe { Ok(*const_try!(self.next_bytes(N)).as_ptr().cast()) }
    }
    pub const unsafe fn next_unsigned(&mut self, size: usize) -> Result<u64> {
        let mut result = 0u64;
        let source = const_try!(self.next_bytes(size));
        unsafe {
            (&raw mut result)
                .cast::<u8>()
                .copy_from_nonoverlapping(source.as_ptr(), source.len());
        }
        Ok(u64::from_le(result))
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

trait Length: for<'a> Decode<'a> {
    fn value(self) -> u64;
}

pub struct Integer<const N: usize>(u64);

unsafe impl<const N: usize> Decode<'_> for Integer<N> {
    fn decode(r: &mut Decoder) -> Result<Self> {
        unsafe { r.next_unsigned(N).map(Self) }
    }
}

impl<const N: usize> Length for Integer<N> {
    fn value(self) -> u64 {
        self.0
    }
}

pub struct Varint<T> {
    value: u64,
    marker: PhantomData<T>,
}

unsafe impl<T> Decode<'_> for Varint<T> {
    fn decode(r: &mut Decoder<'_>) -> Result<Self> {
        let value = r.next_varint_unsigned(mem::size_of::<T>() * 8)?;
        Ok(Self {
            value,
            marker: PhantomData,
        })
    }
}

impl<T> Length for Varint<T> {
    fn value(self) -> u64 {
        self.value
    }
}

pub struct Vector<'a, T, L> {
    ptr: *const u8,
    len: usize,
    marker: PhantomData<(&'a [T], L)>,
}

impl<T, L> Clone for Vector<'_, T, L> {
    fn clone(&self) -> Self {
        Self::new(self.as_bytes())
    }
}

impl<T, L> Copy for Vector<'_, T, L> {}

impl<'a, T, L> Vector<'a, T, L> {
    pub const fn new(data: &'a [u8]) -> Self {
        Self {
            ptr: data.as_ptr(),
            len: data.len(),
            marker: PhantomData,
        }
    }
    pub const fn as_bytes(&self) -> &'a [u8] {
        unsafe { slice::from_raw_parts(self.ptr, self.len) }
    }
    pub const fn len(&self) -> usize {
        self.len / mem::size_of::<T>()
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

impl<'a, T, L> Iterator for Vector<'a, T, L> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<'a, T, L> ExactSizeIterator for Vector<'a, T, L> {
    fn len(&self) -> usize {
        self.len()
    }
}

impl<'a, T, const N: usize> Vector<'a, T, Integer<N>> {
    pub const unsafe fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let len = unsafe { const_try!(r.next_unsigned(N)) };
        let data = const_try!(r.next_bytes(len as usize));
        Ok(Self::new(data))
    }
}

impl<'a, T, I> Vector<'a, T, Varint<I>> {
    pub const unsafe fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let len = const_try!(r.next_varint_unsigned(mem::size_of::<T>()));
        let data = const_try!(r.next_bytes(len as usize));
        Ok(Self::new(data))
    }
}

unsafe impl<'a, T, L: Length> Decode<'a> for Vector<'a, T, L> {
    fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let count: L = r.next()?;
        let len = count.value() as usize * mem::size_of::<T>();
        let data = r.next_bytes(len)?;
        Ok(Self::new(data))
    }
}

impl<T: Debug, L> Debug for Vector<'_, T, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(*self).finish()
    }
}

pub struct Stream<'a, T, L> {
    decoder: Decoder<'a>,
    marker: PhantomData<(T, L)>,
}

impl<T, L> Clone for Stream<'_, T, L> {
    fn clone(&self) -> Self {
        Self {
            decoder: self.decoder,
            marker: PhantomData,
        }
    }
}

impl<T, L> Copy for Stream<'_, T, L> {}

impl<'a, T, L> Stream<'a, T, L> {
    pub const fn new(decoder: Decoder<'a>) -> Self {
        Self {
            decoder,
            marker: PhantomData,
        }
    }
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.decoder.remaining()
    }
}

impl<'a, T, const N: usize> Stream<'a, T, Integer<N>> {
    pub const unsafe fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let len = unsafe { const_try!(r.next_unsigned(N)) };
        let bytes = const_try!(r.next_bytes(len as _));
        Ok(Self::new(Decoder::new(bytes)))
    }
}

impl<'a, T, I> Stream<'a, T, Varint<I>> {
    pub const unsafe fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let len = const_try!(r.next_varint_unsigned(mem::size_of::<T>()));
        let bytes = const_try!(r.next_bytes(len as _));
        Ok(Self::new(Decoder::new(bytes)))
    }
}

impl<'a, T: Decode<'a>, L> Iterator for Stream<'a, T, L> {
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.decoder.remaining_len() == 0 {
            return None;
        }
        Some(self.decoder.next())
    }
}

unsafe impl<'a, T: Decode<'a>, L: Length> Decode<'a> for Stream<'a, T, L> {
    fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let len: L = r.next()?;
        let bytes = r.next_bytes(len.value() as _)?;
        Ok(Self::new(Decoder::new(bytes)))
    }
}

impl<'a, T: Debug + Decode<'a>, L> Debug for Stream<'a, T, L> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list().entries(*self).finish()
    }
}

pub fn decode<'a, T: Decode<'a>>(data: &'a [u8]) -> Result<T> {
    Decoder::new(data).next()
}
