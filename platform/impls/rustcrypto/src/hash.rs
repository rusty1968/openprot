use openprot_hal_blocking::digest::{
    DigestAlgorithm, ErrorType, DigestInit, DigestOp,
    Sha2_256,
    Sha2_384,
    Sha2_512,
    Sha3_256,
    Sha3_384,
    Sha3_512,
};
use sha2::Digest as SwDigest;

pub struct Software;


impl ErrorType for Software {
    type Error = core::convert::Infallible;
}

pub struct Hasher<T> {
    alg: T,
}

impl<T> ErrorType for Hasher<T> {
    type Error = core::convert::Infallible;
}

macro_rules! impl_hashing {
    ($algo:ident, $digest_type:ty, $output_words:expr) => {
        impl DigestInit<$algo> for Software {
            type OpContext<'a> = Hasher<$digest_type>;
            type Output = <$algo as DigestAlgorithm>::Digest;

            fn init<'a>(&'a mut self, _init_params: $algo) -> Result<Self::OpContext<'a>, Self::Error> {
                Ok(Self::OpContext {
                    alg: <$digest_type>::new(),
                })
            }
        }

        impl DigestOp for Hasher<$digest_type> {
            type Output = <$algo as DigestAlgorithm>::Digest;
            
            fn update(&mut self, input: &[u8]) -> Result<(), Self::Error> {
                self.alg.update(input);
                Ok(())
            }
            
            fn finalize(self) -> Result<Self::Output, Self::Error> {
                let output = self.alg.finalize();
                let bytes = output.as_slice();
                
                // Safe conversion: bytes to u32 array with big-endian byte order
                let mut words = [0u32; $output_words];
                for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                    if i < $output_words {
                        words[i] = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    }
                }
                
                Ok(Self::Output { value: words })
            }
        }
    };
}

impl_hashing!(Sha2_256, sha2::Sha256, 8);    // 256 bits = 8 words
impl_hashing!(Sha2_384, sha2::Sha384, 12);   // 384 bits = 12 words  
impl_hashing!(Sha2_512, sha2::Sha512, 16);   // 512 bits = 16 words
impl_hashing!(Sha3_256, sha3::Sha3_256, 8);  // 256 bits = 8 words
impl_hashing!(Sha3_384, sha3::Sha3_384, 12); // 384 bits = 12 words
impl_hashing!(Sha3_512, sha3::Sha3_512, 16); // 512 bits = 16 words

#[cfg(test)]
mod tests {
    use super::Software;
    use openprot_hal_blocking::digest::{
        DigestInit, DigestOp, Sha2_256, Sha2_384, Sha3_256,
    };

    #[test]
    fn test_sha256_empty_string() {
        let mut software = Software;
        let hasher = software.init(Sha2_256).expect("init failed");
        let result = hasher.finalize().expect("finalize failed");
        
        // Expected SHA-256 of empty string
        let expected = [
            0xe3b0c442, 0x98fc1c14, 0x9afbf4c8, 0x996fb924,
            0x27ae41e4, 0x649b934c, 0xa495991b, 0x7852b855,
        ];
        
        assert_eq!(result.value, expected);
    }

    #[test]
    fn test_sha256_hello_world() {
        let mut software = Software;
        let mut hasher = software.init(Sha2_256).expect("init failed");
        hasher.update(b"hello world").expect("update failed");
        let result = hasher.finalize().expect("finalize failed");
        
        // Expected SHA-256 of "hello world"
        let expected = [
            0xb94d27b9, 0x934d3e08, 0xa52e52d7, 0xda7dabfa,
            0xc484efe3, 0x7a5380ee, 0x9088f7ac, 0xe2efcde9,
        ];
        
        assert_eq!(result.value, expected);
    }

    #[test]
    fn test_sha256_streaming() {
        let mut software = Software;
        let mut hasher = software.init(Sha2_256).expect("init failed");
        hasher.update(b"hello").expect("update failed");
        hasher.update(b" ").expect("update failed");
        hasher.update(b"world").expect("update failed");
        let result = hasher.finalize().expect("finalize failed");
        
        // Should be same as "hello world"
        let expected = [
            0xb94d27b9, 0x934d3e08, 0xa52e52d7, 0xda7dabfa,
            0xc484efe3, 0x7a5380ee, 0x9088f7ac, 0xe2efcde9,
        ];
        
        assert_eq!(result.value, expected);
    }

    #[test]
    fn test_sha384_empty_string() {
        let mut software = Software;
        let hasher = software.init(Sha2_384).expect("init failed");
        let result = hasher.finalize().expect("finalize failed");
        
        // Expected SHA-384 of empty string
        let expected = [
            0x38b060a7, 0x51ac9638, 0x4cd9327e, 0xb1b1e36a,
            0x21fdb711, 0x14be0743, 0x4c0cc7bf, 0x63f6e1da,
            0x274edebf, 0xe76f65fb, 0xd51ad2f1, 0x4898b95b,
        ];
        
        assert_eq!(result.value, expected);
    }

    #[test]
    fn test_sha3_256_empty_string() {
        let mut software = Software;
        let hasher = software.init(Sha3_256).expect("init failed");
        let result = hasher.finalize().expect("finalize failed");
        
        // Expected SHA3-256 of empty string
        let expected = [
            0xa7ffc6f8, 0xbf1ed766, 0x51c14756, 0xa061d662,
            0xf580ff4d, 0xe43b49fa, 0x82d80a4b, 0x80f8434a,
        ];
        
        assert_eq!(result.value, expected);
    }

    #[test]
    fn test_digest_as_bytes() {
        let mut software = Software;
        let hasher = software.init(Sha2_256).expect("init failed");
        let result = hasher.finalize().expect("finalize failed");
        
        // Test byte access methods
        let bytes = result.as_bytes();
        assert_eq!(bytes.len(), 32); // 256 bits = 32 bytes
        
        let array = result.into_array();
        assert_eq!(array.len(), 8); // 256 bits = 8 u32 words
    }
}
