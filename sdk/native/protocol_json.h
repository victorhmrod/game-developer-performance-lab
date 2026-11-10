#ifndef GDPL_JSON_H
#define GDPL_JSON_H
#include <ctype.h>
#include <stddef.h>
#include <string.h>
/* Locate a top-level field without mistaking nested fields or string contents for keys. */
static const char *gdpl_field(const char *json, const char *key) {
    const char *p = json;
    int depth = 0;
    while (*p) {
        if (*p == '"') {
            const char *start = ++p;
            while (*p && *p != '"') { if (*p == '\\' && p[1]) ++p; ++p; }
            if (!*p) return NULL;
            const char *end = p++;
            const char *after = p;
            while (isspace((unsigned char)*after)) ++after;
            if (depth == 1 && *after == ':' && (size_t)(end - start) == strlen(key) && !strncmp(start, key, (size_t)(end - start))) {
                p = after + 1;
                while (isspace((unsigned char)*p)) ++p;
                return p;
            }
            continue;
        }
        if (*p == '{' || *p == '[') ++depth;
        if (*p == '}' || *p == ']') --depth;
        if (depth < 0) return NULL;
        ++p;
    }
    return NULL;
}
#endif
