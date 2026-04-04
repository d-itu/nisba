use core::{
    fmt::Debug,
    marker::PhantomData,
    mem::{self, MaybeUninit},
    result, slice,
};

use thiserror::Error;

use crate::{private::Sealed, wrappers::*};

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
    pub const unsafe fn next_varint_unsigned(&mut self, bit_width: usize) -> Result<u64> {
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
    pub const unsafe fn next_varint_signed(&mut self, bit_width: usize) -> Result<i64> {
        let n = unsafe { const_try!(self.next_varint_unsigned(bit_width)) };
        Ok((n >> 1).cast_signed() ^ -((n & 1).cast_signed()))
    }
    pub fn next<T: Decode<'a>>(&mut self) -> Result<T> {
        T::decode(self)
    }
}

pub unsafe trait Decode<'a>: Sized {
    fn decode(r: &mut Decoder<'a>) -> Result<Self>;
}

pub trait Element: Sealed {
    type Output: Sized;
}

pub trait Length: for<'a> Decode<'a> + Sealed {
    fn value(self) -> u64;
}

impl<T> Primitive<T> {
    pub const fn decode(r: &mut Decoder<'_>) -> Result<Self> {
        let mut result = MaybeUninit::uninit();
        let src = const_try!(r.next_bytes(mem::size_of::<T>()));
        unsafe { (&raw mut result as *mut u8).copy_from_nonoverlapping(src.as_ptr(), src.len()) };
        unsafe { result.assume_init() }
    }
}

unsafe impl<T> Decode<'_> for Primitive<T> {
    fn decode(r: &mut Decoder<'_>) -> Result<Self> {
        Self::decode(r)
    }
}

macro_rules! length_primitive {
    ($($t:ty),* $(,)?) => {
        $(impl Length for Primitive<$t> {
            fn value(self) -> u64 {
                self.0 as _
            }
        })*
    };
}
length_primitive!(u8, u16, u32, u64);

impl<T> Element for Primitive<T> {
    type Output = T;
}

unsafe impl<const N: usize> Decode<'_> for Integer<N> {
    fn decode(r: &mut Decoder) -> Result<Self> {
        r.next_u8_array::<N>().map(Self)
    }
}

impl<const N: usize> Element for Integer<N> {
    type Output = [u8; N];
}

impl<const N: usize> Length for Integer<N> {
    fn value(self) -> u64 {
        let mut result = 0;
        unsafe {
            (&raw mut result)
                .cast::<u8>()
                .copy_from_nonoverlapping(self.0.as_ptr(), self.0.len());
        }
        u64::from_le(result)
    }
}

macro_rules! varint_unsigned {
    ($($t:ty),* $(,)?) => {
        $(impl Varint<$t> {
            pub const fn decode(r: &mut Decoder) -> Result<Self> {
                let value = unsafe { const_try!(r.next_varint_unsigned(mem::size_of::<$t>() * 8)) };
                Ok(Self(value as _))
            }
        }
        unsafe impl Decode<'_> for Varint<$t> {
            fn decode(r: &mut Decoder<'_>) -> Result<Self> {
                Self::decode(r)
            }
        }
        impl Length for Varint<$t> {
            fn value(self) -> u64 {
                self.0 as _
            }
        }
        impl Element for Varint<$t> {
            type Output = $t;
        })*
    };
}
varint_unsigned!(u16, u32, u64);

macro_rules! varint_signed {
    ($($t:ty),* $(,)?) => {
        $(impl Varint<$t> {
            pub const fn decode(r: &mut Decoder) -> Result<Self> {
                let value = unsafe { const_try!(r.next_varint_signed(mem::size_of::<$t>() * 8)) };
                Ok(Self(value as _))
            }
        }
        unsafe impl Decode<'_> for Varint<$t> {
            fn decode(r: &mut Decoder<'_>) -> Result<Self> {
                Self::decode(r)
            }
        }
        impl Element for Varint<$t> {
            type Output = $t;
        })*
    };
}
varint_signed!(i16, i32, i64);

pub struct Slice<'a, T, L> {
    ptr: *const u8,
    len: usize,
    marker: PhantomData<(&'a [T], L)>,
}

impl<T, L> Clone for Slice<'_, T, L> {
    fn clone(&self) -> Self {
        Self::new(self.as_bytes())
    }
}

impl<T, L> Copy for Slice<'_, T, L> {}

impl<'a, T, L> Slice<'a, T, L> {
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
    pub fn next(&mut self) -> Option<T::Output>
    where
        T: Element,
    {
        if self.len == 0 {
            return None;
        }
        let ptr: *const T = self.ptr.cast();
        let result = unsafe { ptr.read_unaligned() };
        self.ptr = unsafe { self.ptr.add(mem::size_of::<T>()) };
        self.len -= 1;
        Some(unsafe { (&raw const result).cast::<T::Output>().read() })
    }
}

impl<'a, T: Element, L> Iterator for Slice<'a, T, L> {
    type Item = T::Output;

    fn next(&mut self) -> Option<Self::Item> {
        self.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<'a, T: Element, L> ExactSizeIterator for Slice<'a, T, L> {
    fn len(&self) -> usize {
        self.len()
    }
}

unsafe impl<'a, T, L: Length> Decode<'a> for Slice<'a, T, L> {
    fn decode(r: &mut Decoder<'a>) -> Result<Self> {
        let len: L = r.next()?;
        let size = len.value() as usize * mem::size_of::<T>();
        let data = r.next_bytes(size)?;
        Ok(Self::new(data))
    }
}

impl<T: Element<Output: Debug>, L> Debug for Slice<'_, T, L> {
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
