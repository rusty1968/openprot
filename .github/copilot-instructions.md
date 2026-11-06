## Pull Request Review Checklist

- [ ] Code is completely panic-free (no unwrap/expect/panic/indexing)
- [ ] All fallible operations return Result or Option
- [ ] Integer operations use checked/saturating/wrapping methods where needed
- [ ] Array/slice access uses get() or pattern matching, not direct indexing
- [ ] Error cases are well documented and handled appropriately
- [ ] Tests verify error handling paths, not just happy paths
- [ ] Unsafe code blocks are documented with safety comments
- [ ] Hardware register access uses proper volatile operations
- [ ] Cryptographic operations use constant-time implementations where applicable
- [ ] Code is no_std compatible (no heap allocation: Vec, HashMap, String, etc.)
- [ ] Fixed-size arrays and heapless collections used instead of dynamic allocation
- [ ] Stack usage is bounded and reasonable for embedded targets
- [ ] Shared/reused scratch buffers are not aliased across multiple batched results — each consumer's data is copied or encoded before the buffer is overwritten for the next one

## Quick Reference: Forbidden Patterns

| Forbidden Pattern | Required Alternative |
|-------------------|----------------------|
| `value.unwrap()` | `match value { Some(v) => v, None => return Err(...) }` |
| `result.expect("msg")` | `match result { Ok(v) => v, Err(e) => return Err(e.into()) }` |
| `collection[index]` | `collection.get(index).ok_or(Error::OutOfBounds)?` |
| `a + b` (integers) | `a.checked_add(b).ok_or(Error::Overflow)?` |
| `ptr.read()` | `ptr.read_volatile()` (for MMIO) |
| `Vec<T>`, `HashMap<K,V>` | Fixed-size arrays `[T; N]`, `heapless::Vec<T, N>` |
| `String` | Fixed-size string `heapless::String<N>` or `&str` |
| `Box<T>` | Stack allocation or `&mut T` reference |
| Batching N results against one shared buffer, then processing them after the loop | Copy-and-encode (or fully consume) each result's data immediately, within the same iteration that filled the buffer, before moving to the next |

## Security-Specific Guidelines

- **Timing attacks**: Use constant-time comparisons for secrets (subtle crate)
- **Zeroization**: Use `zeroize` crate for sensitive data cleanup (keys, passwords, etc.)
- **Memory safety**: Ensure sensitive data is properly zeroized after use
- **Hardware abstraction**: All register access must go through HAL traits
- **Error information**: Don't leak sensitive data in error messages
- **Buffer aliasing (CWE-488)**: Never collect metadata for multiple pending/ready items against a single shared buffer and process them afterward — the buffer only ever holds the last write. Each item must be fully encoded/consumed before the buffer is reused for the next.
