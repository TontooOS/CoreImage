/*
 * TontooCoreImage - C Header
 * TontooOS Image Framework (CoreText integrated)
 */

#ifndef TONTOO_COREIMAGE_H
#define TONTOO_COREIMAGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Get the library version string.
 *
 * @return The version string (do NOT free)
 */
const char *coreimage_version(void);

/**
 * Probe image dimensions without full processing.
 *
 * @param path  Input image path
 * @param out_w Receives width
 * @param out_h Receives height
 * @return 1 on success, 0 on error
 */
int coreimage_dimensions(const char *path, uint32_t *out_w, uint32_t *out_h);

/**
 * Blur an image file into a PNG output file.
 *
 * @param input   Input image path
 * @param output  Output PNG path
 * @param sigma   Blur sigma (> 0)
 * @param err_out Receives an error string on failure (free with coreimage_string_free)
 * @return 0 on success, negative on error
 */
int coreimage_blur_to_file(const char *input, const char *output, float sigma, char **err_out);

/**
 * Free a string returned by CoreImage.
 *
 * @param ptr The string to free
 */
void coreimage_string_free(char *ptr);

#ifdef __cplusplus
}
#endif

#endif /* TONTOO_COREIMAGE_H */
