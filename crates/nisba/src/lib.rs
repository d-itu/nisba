#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

#[macro_export]
macro_rules! const_try {
    ($x:expr) => {
        match $x {
            Ok(x) => x,
            Err(e) => return Err(e),
        }
    };
}

pub use static_assertions::const_assert_eq;

pub mod decode;
pub mod encode;
