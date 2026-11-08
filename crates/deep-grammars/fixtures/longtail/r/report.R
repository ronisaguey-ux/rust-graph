# Build a small markdown report from the JSON summary.
read_summary <- function(path) {
  stopifnot(file.exists(path))
  jsonlite::fromJSON(path)
}

format_row <- function(name, value) {
  sprintf("| %-20s | %10s |", name, format(value, big.mark = ","))
}

report <- function(summary) {
  lines <- c(
    "## Scanner summary",
    "",
    "| metric               |      value |",
    "|----------------------|------------|",
    format_row("files", summary$count),
    format_row("bytes", summary$bytes)
  )
  paste(lines, collapse = "\n")
}

main <- function() {
  summary <- read_summary("summary.json")
  text <- report(summary)
  writeLines(text, "summary.md")
  invisible(text)
}

if (sys.nframe() == 0) main()
