/* The Tree-sitter side of the parser gate (tests/parser_gate.rs). Built by
 * bench/build-tsdrv.sh against pinned Tree-sitter and grammar sources.
 *
 *   tsdrv bench LANG LIST [REPS]   every file in LIST read first, then each parsed
 *                                  REPS times by a fresh parser; the best counts
 *   tsdrv sexp  LANG FILE          the tree as an S-expression
 *
 * LANG is the grammar's symbol suffix: python, rust, javascript, typescript,
 * tsx, go, ... The grammar is found with dlsym, so adding one is a link flag.
 * `bench` prints one line of key=value pairs; the gate reads best_ms and
 * clean_files from it. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <tree_sitter/api.h>

static const TSLanguage *pick(const char *name) {
  char symbol[128];
  snprintf(symbol, sizeof symbol, "tree_sitter_%s", name);
  const TSLanguage *(*make)(void) = (const TSLanguage *(*)(void))dlsym(RTLD_DEFAULT, symbol);
  if (!make) {
    fprintf(stderr, "tsdrv: no grammar %s linked in (bench/build-tsdrv.sh)\n", symbol);
    exit(2);
  }
  return make();
}

static double now(void) {
  struct timespec t;
  clock_gettime(CLOCK_MONOTONIC, &t);
  return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static char *slurp(const char *path, size_t *n) {
  FILE *f = fopen(path, "rb");
  if (!f) return 0;
  fseek(f, 0, SEEK_END);
  *n = ftell(f);
  rewind(f);
  char *bytes = malloc(*n + 1);
  *n = fread(bytes, 1, *n, f);
  fclose(f);
  return bytes;
}

/* Bytes under outermost ERROR or MISSING nodes, as the deep scan counts them. */
static void errstats(TSNode root, unsigned long *error_bytes, unsigned long *nodes) {
  TSTreeCursor c = ts_tree_cursor_new(root);
  for (;;) {
    TSNode n = ts_tree_cursor_current_node(&c);
    (*nodes)++;
    if (ts_node_is_error(n) || ts_node_is_missing(n)) {
      *error_bytes += ts_node_end_byte(n) - ts_node_start_byte(n);
    } else if (ts_tree_cursor_goto_first_child(&c)) {
      continue;
    }
    if (ts_tree_cursor_goto_next_sibling(&c)) continue;
    int up = 0;
    while (ts_tree_cursor_goto_parent(&c))
      if (ts_tree_cursor_goto_next_sibling(&c)) { up = 1; break; }
    if (!up) break;
  }
  ts_tree_cursor_delete(&c);
}

int main(int argc, char **argv) {
  if (argc < 4) {
    fprintf(stderr, "usage: tsdrv bench LANG LIST [REPS] | sexp LANG FILE\n");
    return 2;
  }
  const TSLanguage *language = pick(argv[2]);
  if (!strcmp(argv[1], "sexp")) {
    size_t n;
    char *bytes = slurp(argv[3], &n);
    if (!bytes) { fprintf(stderr, "tsdrv: cannot read %s\n", argv[3]); return 2; }
    TSParser *p = ts_parser_new();
    ts_parser_set_language(p, language);
    TSTree *t = ts_parser_parse_string(p, 0, bytes, n);
    char *s = ts_node_string(ts_tree_root_node(t));
    printf("%s\n", s);
    return 0;
  }
  if (strcmp(argv[1], "bench")) { fprintf(stderr, "tsdrv: unknown command %s\n", argv[1]); return 2; }
  int reps = argc > 4 ? atoi(argv[4]) : 5;
  if (reps < 1) reps = 1;
  FILE *list = fopen(argv[3], "r");
  if (!list) { fprintf(stderr, "tsdrv: cannot read %s\n", argv[3]); return 2; }
  char line[4096];
  int cap = 64, files = 0;
  char **bytes = malloc(cap * sizeof *bytes);
  size_t *sizes = malloc(cap * sizeof *sizes), total = 0;
  while (fgets(line, sizeof line, list)) {
    line[strcspn(line, "\n")] = 0;
    if (!line[0]) continue;
    if (files == cap) {
      cap *= 2;
      bytes = realloc(bytes, cap * sizeof *bytes);
      sizes = realloc(sizes, cap * sizeof *sizes);
    }
    bytes[files] = slurp(line, &sizes[files]);
    if (!bytes[files]) { fprintf(stderr, "tsdrv: cannot read %s\n", line); return 2; }
    total += sizes[files++];
  }
  double best = 0, first = 0;
  int clean = 0;
  unsigned long error_bytes = 0, nodes = 0;
  for (int i = 0; i < files; i++) {
    double fastest = 1e18;
    for (int r = 0; r < reps; r++) {
      TSParser *p = ts_parser_new();
      ts_parser_set_language(p, language);
      double start = now();
      TSTree *t = ts_parser_parse_string(p, 0, bytes[i], sizes[i]);
      double elapsed = now() - start;
      if (elapsed < fastest) fastest = elapsed;
      if (r == 0) first += elapsed;
      if (r == reps - 1) {
        TSNode root = ts_tree_root_node(t);
        if (!ts_node_has_error(root)) clean++;
        errstats(root, &error_bytes, &nodes);
      }
      ts_tree_delete(t);
      ts_parser_delete(p);
    }
    best += fastest;
  }
  printf("%s files=%d bytes=%zu best_ms=%.1f first_pass_ms=%.1f MB_s=%.1f clean_files=%d/%d err_bytes=%lu (%.3f%%) nodes=%lu\n",
         argv[2], files, total, best, first, total / 1e6 / (best / 1e3), clean, files, error_bytes,
         100.0 * error_bytes / (total ? total : 1), nodes);
  return 0;
}
