#ifndef GDPL_JSON_H
#define GDPL_JSON_H
#ifndef _WIN32
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif
#endif
#include <ctype.h>
#include <stddef.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#include <io.h>
#else
#include <poll.h>
#include <unistd.h>
#include <time.h>
#endif
/* Read a complete bounded line using an absolute timeout; a partial line is never accepted. */
static int gdpl_read_line(char *buffer, size_t capacity) {
    size_t used = 0;
#ifdef _WIN32
    ULONGLONG deadline = GetTickCount64() + 5000;
    HANDLE input = (HANDLE)_get_osfhandle(0);
    while (GetTickCount64() < deadline && used + 1 < capacity) {
        DWORD available = 0;
        if (!PeekNamedPipe(input, NULL, 0, NULL, &available, NULL)) return 0;
        if (!available) { Sleep(5); continue; }
        if (_read(0, buffer + used, 1) != 1) return 0;
        if (!buffer[used]) return 0;
        if (++used > 8192) return 0;
        if (buffer[used - 1] == '\n') { buffer[used] = 0; return 1; }
    }
#else
    struct timespec now;
    if (clock_gettime(CLOCK_MONOTONIC, &now)) return 0;
    long long deadline = (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000 + 5000;
    while (used + 1 < capacity) {
        if (clock_gettime(CLOCK_MONOTONIC, &now)) return 0;
        long long remaining = deadline - ((long long)now.tv_sec * 1000 + now.tv_nsec / 1000000);
        if (remaining <= 0) return 0;
        struct pollfd descriptor = {0, POLLIN, 0};
        if (poll(&descriptor, 1, (int)remaining) <= 0 || read(0, buffer + used, 1) != 1) return 0;
        if (!buffer[used]) return 0;
        if (++used > 8192) return 0;
        if (buffer[used - 1] == '\n') { buffer[used] = 0; return 1; }
    }
#endif
    return 0;
}
/* Validate JSON framing and grammar before inspecting envelope fields. */
static void gdpl_space(const char **p) { while (**p == ' ' || **p == '\n' || **p == '\r' || **p == '\t') ++*p; }
static int gdpl_string(const char **p) {
    if (*(*p)++ != '"') return 0;
    while (**p && **p != '"') {
        unsigned char ch = (unsigned char)*(*p)++;
        if (ch < 0x20) return 0;
        if (ch == '\\') {
            char escape = *(*p)++;
            if (!escape) return 0;
            if (escape == 'u') {
                for (int i = 0; i < 4; ++i) { if (!isxdigit((unsigned char)**p)) return 0; ++*p; }
            } else if (!strchr("\"\\/bfnrt", escape)) return 0;
        }
    }
    if (**p != '"') return 0;
    ++*p;
    return 1;
}
static int gdpl_value(const char **p, int depth) {
    gdpl_space(p);
    if (depth > 64 || !**p) return 0;
    if (**p == '"') return gdpl_string(p);
    if (**p == '{' || **p == '[') {
        int object = *(*p)++ == '{';
        char close = object ? '}' : ']';
        gdpl_space(p);
        if (**p == close) { ++*p; return 1; }
        for (;;) {
            if (object) {
                if (**p != '"' || !gdpl_string(p)) return 0;
                gdpl_space(p);
                if (**p != ':') return 0;
                ++*p;
            }
            if (!gdpl_value(p, depth + 1)) return 0;
            gdpl_space(p);
            if (**p == close) { ++*p; return 1; }
            if (**p != ',') return 0;
            ++*p;
            gdpl_space(p);
        }
    }
    if (!strncmp(*p, "true", 4)) { *p += 4; return 1; }
    if (!strncmp(*p, "false", 5)) { *p += 5; return 1; }
    if (!strncmp(*p, "null", 4)) { *p += 4; return 1; }
    if (**p == '-') ++*p;
    if (**p == '0') ++*p;
    else {
        if (**p < '1' || **p > '9') return 0;
        while (isdigit((unsigned char)**p)) ++*p;
    }
    if (**p == '.') {
        ++*p;
        if (!isdigit((unsigned char)**p)) return 0;
        while (isdigit((unsigned char)**p)) ++*p;
    }
    if (**p == 'e' || **p == 'E') {
        ++*p;
        if (**p == '+' || **p == '-') ++*p;
        if (!isdigit((unsigned char)**p)) return 0;
        while (isdigit((unsigned char)**p)) ++*p;
    }
    return 1;
}
static int gdpl_valid_json(const char *p) {
    gdpl_space(&p);
    if (*p != '{' || !gdpl_value(&p, 0)) return 0;
    gdpl_space(&p);
    return *p == 0;
}
/* Locate a top-level field without mistaking nested fields or string contents for keys. */
static const char *gdpl_field(const char *json, const char *key) {
    const char *p = json;
    int depth = 0;
    const char *found = NULL;
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
                if (found) return NULL;
                found = p;
            }
            continue;
        }
        if (*p == '{' || *p == '[') ++depth;
        if (*p == '}' || *p == ']') --depth;
        if (depth < 0) return NULL;
        ++p;
    }
    return found;
}
#endif
