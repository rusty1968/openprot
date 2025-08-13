# AST1060 Starter Task

A minimal ExHubris task for AST1060-based applications. This serves as a starting point for developing applications on AST1060 hardware using the ExHubris microkernel.

## Features

- **Basic task structure**: Demonstrates proper ExHubris task pattern with `#[export_name = "main"]`
- **JTAG debugging**: Optional halt feature for debugger attachment
- **Hardware abstraction**: Uses kernel-level hardware initialization instead of direct register access
- **Clean separation**: Application logic separated from hardware setup

## Architecture

This task follows the ExHubris model:

- **Hardware initialization** happens in the kernel (`kernel-generic-ast1060`)
- **Application logic** runs as isolated tasks (this task)
- **No direct hardware access** - tasks use IPC to request services from driver tasks

## Usage

This task would typically be included in an ExHubris application configuration (`app.kdl`) alongside other tasks and drivers:

```kdl
app ast1060-demo

board "proj:boards/ast1060-generic.kdl"

kernel {
    workspace-crate kernel-generic-ast1060
    stack-size 1024
}

task starter {
    workspace-crate ast1060-starter
    stack-size 512
    priority 3
}

// Other tasks would be defined here...
```

## Development

- Hardware setup (clocks, crypto peripherals, JTAG pins) is handled by `kernel-generic-ast1060`
- This task focuses on application-level functionality
- For crypto operations, tasks would communicate with crypto driver tasks via IPC
