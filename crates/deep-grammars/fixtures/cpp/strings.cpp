#include <string>

const char *raw_text() {
    return R"(a raw string with "quotes" and \backslashes)";
}

const char *tagged_raw() {
    return R"sql(select * from "t" where x = 'y')sql";
}

const char *wide_text() {
    return L"wide \t escape";
}

int escaping() {
    char newline = '\n';
    char tab = '\t';
    char quote = '\'';
    char hex = '\x41';
    char octal = '\101';
    unsigned long big = 1'000'000UL;
    double fraction = 3.5e-2;
    float small = 0.5f;
    int binary = 0b1010;
    int hexed = 0xFF;
    return newline + tab + quote + hex + octal + binary + hexed + (int)big + (int)(fraction + small);
}

int concatenated() {
    const char *path = "dir" "/" "file" ".txt";
    return (int)sizeof(path);
}
