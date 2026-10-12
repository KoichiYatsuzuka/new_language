using ArrowHost;

// Arrow cs-proc host for cs_probe (implementation_plans/ffi_value_exchange_plan.md, task 1-1).
// Registers all public types in this assembly and runs the IPC loop.
var host = new ArrowPipeHost(typeof(Probe).Assembly);
host.Run(args);
