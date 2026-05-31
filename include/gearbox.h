#ifndef GEARBOX_H
#define GEARBOX_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

const char *gearbox_last_error_message(void);

void gearbox_string_free(char *s);

char *gearbox_version(void);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* GEARBOX_H */
