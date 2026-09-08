## Concurrency Model

Each service is a single-threaded userspace process. Services communicate only
through message passing over IPC channels. The architecture does not share
memory across processes or transfer values between threads.

- Do not add `Send` or `Sync` supertrait bounds to traits unless a documented
  API specifically requires cross-thread transfer or shared access.
- Do not add speculative concurrency bounds for hypothetical future use.
- Preserve intentionally thread-affine resource-holder types.
- Plain data and message types should remain neutral. Do not explicitly add or
  remove marker traits without a concrete API requirement.
- If an existing `Send` or `Sync` bound appears unnecessary, flag it for review
  rather than silently propagating it to additional interfaces.
