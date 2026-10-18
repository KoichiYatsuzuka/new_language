// linux_lib.h - a header whose directory says "os": "linux" in ar_config.json (ffi_value_exchange_plan.md task 7-32).
// On Windows, importing it is an error: a C library built for Linux cannot be loaded here.
#pragma once
long ll_size(void);
