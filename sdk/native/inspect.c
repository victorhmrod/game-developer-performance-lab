#include "protocol_json.h"
#include <stdio.h>
#include <stdlib.h>
int main(void) {
    const char *commands[] = {"hello", "list_scenarios", "shutdown"};
    char line[8194];
    for (int i = 0; i < 3; ++i) {
        printf("{\"version\":1,\"id\":%d,\"command\":\"%s\",\"data\":%s}\n", i + 1, commands[i], i == 0 ? "{\"versions\":[1]}" : "{}");
        fflush(stdout);
        if (!gdpl_read_line(line, sizeof line)) { fprintf(stderr, "disconnected or oversized reply\n"); return 1; }
        if (!gdpl_valid_json(line)) { fprintf(stderr, "malformed JSON\n"); return 1; }
        const char *id = gdpl_field(line, "id"), *version = gdpl_field(line, "version"), *ok = gdpl_field(line, "ok");
        if (!id || !version || !ok || strtod(id, NULL) != i + 1 || strtod(version, NULL) != 1 || strncmp(ok, "true", 4)) {
            const char *code = gdpl_field(line, "error_code");
            fprintf(stderr, "invalid response or protocol error; code: %s\n", code ? code : "unavailable"); return 1;
        }
        if (i == 0 && (!gdpl_field(line, "capabilities") || *gdpl_field(line, "capabilities") != '[' || !gdpl_field(line, "metric_definitions") || *gdpl_field(line, "metric_definitions") != '{')) { fprintf(stderr, "missing discovery data\n"); return 1; }
        if (i == 1 && (!gdpl_field(line, "scenarios") || *gdpl_field(line, "scenarios") != '[')) { fprintf(stderr, "missing scenarios\n"); return 1; }
        fprintf(stderr, "%s accepted: %s", commands[i], line);
    }
    return 0;
}
