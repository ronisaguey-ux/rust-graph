#!/usr/bin/env Rscript

args <- commandArgs(trailingOnly = TRUE)
if (length(args) == 0) {
  stop("usage: pipeline.R <root> [language]")
}
root <- args[1]
language <- if (length(args) > 1) args[2] else "rust"

walk <- function(dir, depth = 0) {
  if (depth > 8) return(character())
  entries <- list.files(dir, full.names = TRUE)
  files <- character()
  while (length(entries) > 0) {
    entry <- entries[1]
    entries <- entries[-1]
    if (dir.exists(entry)) {
      entries <- c(entries, list.files(entry, full.names = TRUE))
    } else {
      files <- c(files, entry)
    }
  }
  files
}

matches <- Filter(function(path) grepl(paste0("\\.", language, "$"), path), walk(root))
message(sprintf("found %d %s files", length(matches), language))

sizes <- file.size(matches)
cat(jsonlite::toJSON(list(count = length(matches), bytes = sum(sizes)), auto_unbox = TRUE), "\n")
