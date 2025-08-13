// OpenPRoT HAL trait implementations for Aspeed HACE hash operations
use core::convert::Infallible;
use openprot_hal_blocking::digest::{
    DigestAlgorithm, DigestInit, DigestOp, DigestCtrlReset, ErrorType,
    Sha2_256, Sha2_384, Sha2_512, Digest
};
use crate::{HaceController, HashAlgo};
use crate::hash::ContextCleanup;

// Error type implementation
impl ErrorType for HaceController {
    type Error = Infallible;
}

// Hash context wrapper that implements DigestOp
pub struct HashContext<'a, T> {
    controller: &'a mut HaceController,
    _alg: T,
}

impl<T> ErrorType for HashContext<'_, T> {
    type Error = Infallible;
}

// Implement DigestOp specifically for each algorithm
impl DigestOp for HashContext<'_, Sha2_256> {
    type Output = Digest<8>; // 256 bits / 32 = 8 words
    
    fn update(&mut self, input: &[u8]) -> Result<(), Self::Error> {
        <HashContext<'_, Sha2_256> as HashContextOps>::update_impl(self, input)
    }
    
    fn finalize(self) -> Result<Self::Output, Self::Error> {
        // Add padding and process final block
        self.controller.fill_padding(0);
        
        let hash_cmd = self.controller.algo.hash_cmd();
        let bufcnt = {
            let ctx = self.controller.ctx_mut();
            ctx.method = hash_cmd;
            ctx.bufcnt
        };
        self.controller.start_hash_operation(bufcnt);
        
        let digest_array: [u32; 8] = self.controller.digest();
        Ok(Digest { value: digest_array })
    }
}

impl DigestOp for HashContext<'_, Sha2_384> {
    type Output = Digest<12>; // 384 bits / 32 = 12 words
    
    fn update(&mut self, input: &[u8]) -> Result<(), Self::Error> {
        <HashContext<'_, Sha2_384> as HashContextOps>::update_impl(self, input)
    }
    
    fn finalize(self) -> Result<Self::Output, Self::Error> {
        // Add padding and process final block
        self.controller.fill_padding(0);
        
        let hash_cmd = self.controller.algo.hash_cmd();
        let bufcnt = {
            let ctx = self.controller.ctx_mut();
            ctx.method = hash_cmd;
            ctx.bufcnt
        };
        self.controller.start_hash_operation(bufcnt);
        
        let digest_array: [u32; 12] = self.controller.digest();
        Ok(Digest { value: digest_array })
    }
}

impl DigestOp for HashContext<'_, Sha2_512> {
    type Output = Digest<16>; // 512 bits / 32 = 16 words
    
    fn update(&mut self, input: &[u8]) -> Result<(), Self::Error> {
        <HashContext<'_, Sha2_512> as HashContextOps>::update_impl(self, input)
    }
    
    fn finalize(self) -> Result<Self::Output, Self::Error> {
        // Add padding and process final block
        self.controller.fill_padding(0);
        
        let hash_cmd = self.controller.algo.hash_cmd();
        let bufcnt = {
            let ctx = self.controller.ctx_mut();
            ctx.method = hash_cmd;
            ctx.bufcnt
        };
        self.controller.start_hash_operation(bufcnt);
        
        let digest_array: [u32; 16] = self.controller.digest();
        Ok(Digest { value: digest_array })
    }
}

// Helper trait to share the update implementation
trait HashContextOps {
    fn update_impl(&mut self, input: &[u8]) -> Result<(), Infallible>;
}

impl<T> HashContextOps for HashContext<'_, T> {
    fn update_impl(&mut self, input: &[u8]) -> Result<(), Infallible> {
        let input_len = input.len();
        
        // Update digest count
        {
            let ctx = self.controller.ctx_mut();
            let old_count = ctx.digcnt[0];
            ctx.digcnt[0] += input_len as u64;
            if ctx.digcnt[0] < old_count {
                ctx.digcnt[1] += 1; // Handle overflow
            }
        }
        
        // Process input in chunks that fit in the buffer
        let mut remaining = input;
        while !remaining.is_empty() {
            let (available_space, bufcnt, buffer_full) = {
                let ctx = self.controller.ctx_mut();
                let available_space = ctx.buffer.len() - ctx.bufcnt as usize;
                let to_copy = core::cmp::min(remaining.len(), available_space);
                
                // Copy data to buffer
                let start = ctx.bufcnt as usize;
                ctx.buffer[start..start + to_copy].copy_from_slice(&remaining[..to_copy]);
                ctx.bufcnt += to_copy as u32;
                
                let buffer_full = ctx.bufcnt as usize == ctx.buffer.len();
                (available_space, ctx.bufcnt, buffer_full)
            };
            
            let to_copy = core::cmp::min(remaining.len(), available_space);
            remaining = &remaining[to_copy..];
            
            // If buffer is full, process it
            if buffer_full {
                let hash_cmd = self.controller.algo.hash_cmd();
                {
                    let ctx = self.controller.ctx_mut();
                    ctx.method = hash_cmd;
                }
                self.controller.start_hash_operation(bufcnt);
                {
                    let ctx = self.controller.ctx_mut();
                    ctx.bufcnt = 0;
                }
            }
        }
        
        Ok(())
    }
}

// Macro to generate DigestInit implementations similar to OpenTitan
macro_rules! impl_sha2 {
    ($algo:ident, $hash_algo:expr) => {
        impl DigestInit<$algo> for HaceController {
            type OpContext<'a> = HashContext<'a, $algo> where Self: 'a;
            type Output = <$algo as DigestAlgorithm>::Digest;
            
            fn init<'a>(&'a mut self, init_params: $algo) -> Result<Self::OpContext<'a>, Self::Error> {
                // Store algorithm values to avoid borrowing conflicts
                let hash_cmd = $hash_algo.hash_cmd();
                let block_size = $hash_algo.block_size() as u32;
                let iv_size = $hash_algo.iv_size() as u8;
                
                self.algo = $hash_algo;
                self.cleanup_context();
                
                {
                    let ctx = self.ctx_mut();
                    ctx.method = hash_cmd;
                    ctx.block_size = block_size;
                    ctx.iv_size = iv_size;
                }
                
                // Initialize digest with algorithm-specific IV
                self.copy_iv_to_digest();
                
                Ok(HashContext { 
                    controller: self,
                    _alg: init_params,
                })
            }
        }
    };
}

impl_sha2!(Sha2_256, HashAlgo::SHA256);
impl_sha2!(Sha2_384, HashAlgo::SHA384);
impl_sha2!(Sha2_512, HashAlgo::SHA512);

// DigestCtrlReset implementation
impl DigestCtrlReset for HaceController {
    fn reset(&mut self) -> Result<(), Self::Error> {
        self.cleanup_context();
        Ok(())
    }
}
