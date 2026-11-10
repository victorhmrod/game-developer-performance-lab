#include "protocol_json.h"
#include <cstdlib>
#include <iostream>
#include <string>
int main() {
    const char *commands[] = {"hello", "list_scenarios", "shutdown"};
    for (int i = 0; i < 3; ++i) {
        std::cout << "{\"version\":1,\"id\":" << i + 1 << ",\"command\":\"" << commands[i] << "\",\"data\":" << (i == 0 ? "{\"versions\":[1]}" : "{}") << "}" << std::endl;
        char buffer[8194];
        if (!gdpl_read_line(buffer, sizeof buffer)) { std::cerr << "disconnected or oversized reply\n"; return 1; }
        std::string line(buffer);
        if (!gdpl_valid_json(line.c_str())) { std::cerr << "malformed JSON\n"; return 1; }
        const char *id = gdpl_field(line.c_str(), "id"), *version = gdpl_field(line.c_str(), "version"), *ok = gdpl_field(line.c_str(), "ok");
        if (!id || !version || !ok || std::strtod(id, nullptr) != i + 1 || std::strtod(version, nullptr) != 1 || std::strncmp(ok, "true", 4)) {
            const char *code = gdpl_field(line.c_str(), "error_code");
            std::cerr << "invalid response or protocol error; code: " << (code ? code : "unavailable") << '\n'; return 1;
        }
        if (i == 0 && (!gdpl_field(line.c_str(), "capabilities") || *gdpl_field(line.c_str(), "capabilities") != '[' || !gdpl_field(line.c_str(), "metric_definitions") || *gdpl_field(line.c_str(), "metric_definitions") != '{')) { std::cerr << "missing discovery data\n"; return 1; }
        if (i == 1 && (!gdpl_field(line.c_str(), "scenarios") || *gdpl_field(line.c_str(), "scenarios") != '[')) { std::cerr << "missing scenarios\n"; return 1; }
        std::cerr << commands[i] << " accepted: " << line << '\n';
    }
    return 0;
}
