#![no_std]
mod driver;

use base::println;
use ufmt;

use driver::*;
pub use driver::Hmac;
use hmac::enums::DigestSize;

use traits::digest::{
    Digest, DigestAlgorithm, ErrorKind, Error, ErrorType, DigestInit, DigestOp,
    Sha2_256,
    Sha2_384,
    Sha2_512,
};

#[derive(Debug)]
pub struct HmacError(ErrorKind);

impl Error for HmacError {
    fn kind(&self) -> ErrorKind {
        self.0
    }
}

impl ErrorType for Hmac {
    type Error = HmacError;
}

pub struct Hasher<'a, T> {
    hw: &'a mut Hmac,
    _alg: T,
}

impl<T> ErrorType for Hasher<'_, T> {
    type Error = HmacError;
}

macro_rules! impl_sha2 {
    ($algo:ident, $digest_size:expr) => {
        impl DigestInit<$algo> for Hmac {
            type OpContext<'a> = Hasher<'a, $algo>;
            type Output = <$algo as DigestAlgorithm>::Digest;

            fn init<'a>(&'a mut self, init_params: $algo) -> Result<Self::OpContext<'a>, Self::Error> {
                self.configure($digest_size);
                self.start();
                Ok(Self::OpContext {
                    hw: self,
                    _alg: init_params,
                })
            }
        }

        impl DigestOp for Hasher<'_, $algo> {
            type Output = <$algo as DigestAlgorithm>::Digest;
            fn update(&mut self, input: &[u8]) -> Result<(), Self::Error> {
                self.hw.update(input);
                Ok(())
            }
            fn finalize(self) -> Result<Self::Output, Self::Error> {
                self.hw.process();
                self.hw.wait_for_done();
                Ok(Self::Output {
                    value: self.hw.digest(),
                })
            }
        }
    };
}

impl_sha2!(Sha2_256, DigestSize::Sha2256);
impl_sha2!(Sha2_384, DigestSize::Sha2384);
impl_sha2!(Sha2_512, DigestSize::Sha2512);