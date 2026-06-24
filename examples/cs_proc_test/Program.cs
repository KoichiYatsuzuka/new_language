using ArrowHost;

// Arrow cs-proc host for cs_proc_test.
// Registers all public types in this assembly and runs the IPC loop.
var host = new ArrowPipeHost(typeof(Calculator).Assembly);
host.Run(args);
