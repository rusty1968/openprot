The AST10x0 SMC user-mode command path is still using the SMC hardware as the SPI master. A separate software SPI engine would mean your firmware itself is emulating SPI transactions outside that SMC command path, typically by bit-banging GPIOs or building a second independent controller abstraction that manually toggles clock/data semantics.

What “user mode command path” means here:

The SMC is temporarily switched out of its normal memory-mapped read mode and into a command-transaction mode for a selected chip select.
In that mode, writes to the flash aperture are interpreted by the SMC hardware as outgoing SPI command/address/data bytes.
Reads from that aperture are interpreted as incoming SPI response/data bytes.
When the transaction is done, the controller is switched back to normal read mode so the usual mapped-window read path continues to work.
So the hardware still does:

Chip-select assertion/deassertion.
SPI clock generation.
Serial shifting of command/address/data bits.
Interpretation of the aperture as a transaction stream while user mode is active.
What a “separate software SPI engine” would mean instead:

A second code path that does not rely on the SMC’s user transaction mechanism.
Firmware manually sequencing SPI semantics itself, either by GPIO bit-bang or by inventing a second transport layer unrelated to the SMC aperture/user-mode model.
Duplicating concerns the SMC already owns: CS timing, shifting, data streaming rules, and command/data phases.
Why the plan says not to do that:

The SMC already has a hardware path for command transactions.
aspeed-rust shows that path working by activating user mode and streaming bytes through the mapped window.
Reusing that path keeps one hardware authority for reads and writes.
A separate software engine would duplicate controller behavior, increase audit surface, and make correctness harder to reason about.
A simple mental model:

Normal mode: the SMC treats the flash aperture like memory-backed read space.
User mode: the same aperture becomes a byte-stream port for explicit SPI transactions.
If you want stronger wording in the plan, this is a cleaner sentence:

“Use the AST10x0 SMC’s built-in user-mode transaction path, where the flash aperture acts as the command/data stream while the SMC hardware continues to drive CS and SPI shifting; do not introduce a second firmware-owned SPI transport or GPIO bit-bang engine.”

If you want, I can patch that wording directly into target/ast10x0/peripherals/smc/planning/SMC_FLASH_COMMAND_TRANSPORT_PLAN.md.

GPT-5.4 • 0.9x