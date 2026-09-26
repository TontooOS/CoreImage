# Ffi

C ABI in `Headers/coreimage.h` implemented by `src/ffi.rs`. The full API lives
in Rust; C hosts get version, dimension probe and a blur helper.

## Functions

### `coreimage_version`

```c
const char *coreimage_version(void);
```

| Return | Meaning |
|---|---|
| pointer | Borrowed version string, do not free |

### `coreimage_dimensions`

```c
int coreimage_dimensions(const char *path, uint32_t *out_w, uint32_t *out_h);
```

| Return | Meaning |
|---|---|
| 1 | Success, dimensions written |
| 0 | Null pointer or unreadable image |

### `coreimage_blur_to_file`

```c
int coreimage_blur_to_file(const char *input, const char *output, float sigma, char **err_out);
```

| Return | Meaning |
|---|---|
| 0 | Success, PNG written |
| -1 | Null input/output pointer |
| -2 | Load, blur or save failed, `err_out` set when non-null |

### `coreimage_string_free`

```c
void coreimage_string_free(char *ptr);
```

Frees strings produced by CoreImage. Null-safe.

## Memory Rules

| Value | Rule |
|---|---|
| `coreimage_version` result | Borrowed, never free |
| `err_out` string | Caller frees with `coreimage_string_free` |
| Null `err_out` | Allowed, error text is dropped |

## Usage / Example

```c
#include "coreimage.h"

char *err = 0;
int rc = coreimage_blur_to_file("in.png", "dock.png", 8.0f, &err);
if (rc != 0 && err) {
  coreimage_string_free(err);
}
```

## Cross References

- [Composite.md](Composite.md) – Rust blur and overlay behind the helper
- [LoadingSaving.md](LoadingSaving.md) – Rust load/save API
