// ffi_probe_extra.h - included by ffi_probe.h (ffi_value_exchange_plan.md task 7-28).
// import[cpp-lib] also reads the local headers a header includes (when cpp.precompile_macros is set),
// and the type stub must see the same functions as the runtime.
#pragma once
#ifdef __cplusplus
extern "C" {
#endif
int fp_extra(int x);   // x * 3
#ifdef __cplusplus
}
#endif
